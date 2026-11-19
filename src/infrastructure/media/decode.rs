//! Bounded decoding of PNG, JPEG and GIF wallpaper (contract §14.2): magic detection, source and
//! canvas ceilings, composited animation frames, aspect-preserving downscale without upscaling, and
//! checked memory charges before each frame is retained.

use std::io::Cursor;
use std::sync::Arc;

use image::codecs::gif::GifDecoder;
use image::imageops::{FilterType, resize};
use image::{AnimationDecoder, DynamicImage, ImageDecoder, ImageFormat, Limits, RgbaImage};

use super::{
    DecodeRequest, DecodedMedia, MAX_AXIS, MAX_FRAMES, MAX_SOURCE_BYTES, MediaError, MediaFrame,
    Repeat, Source,
};
use crate::domain::appearance::Quality;

/// A normalized GIF delay of 10 ms or less (browsers do the same) becomes 100 ms.
pub const MIN_GIF_DELAY_MS: u32 = 10;
pub const REPLACEMENT_DELAY_MS: u32 = 100;

/// The three accepted source formats, recognized by magic bytes only.
#[must_use]
pub fn detect(bytes: &[u8]) -> Option<ImageFormat> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(ImageFormat::Png)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(ImageFormat::Jpeg)
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(ImageFormat::Gif)
    } else {
        None
    }
}

/// The per-axis ceiling for a quality tier.
#[must_use]
pub fn quality_cap(quality: Quality) -> (u32, u32) {
    match quality {
        Quality::Auto => (1920, 1080),
        Quality::Low => (960, 540),
        Quality::High => (2048, 2048),
    }
}

/// The prepared size: the source fitted inside the smaller of the display extent (1920×1080 when
/// unknown) and the quality cap, keeping its aspect ratio and never upscaling.
#[must_use]
pub fn target_size(
    width: u32,
    height: u32,
    display: Option<(u32, u32)>,
    quality: Quality,
) -> (u32, u32) {
    let (dw, dh) = display.unwrap_or((1920, 1080));
    let (cw, ch) = quality_cap(quality);
    let bound_w = u64::from(dw.min(cw).max(1));
    let bound_h = u64::from(dh.min(ch).max(1));
    let (w, h) = (u64::from(width.max(1)), u64::from(height.max(1)));
    if w <= bound_w && h <= bound_h {
        return (width.max(1), height.max(1));
    }
    let (nw, nh) = if w * bound_h >= h * bound_w {
        (bound_w, ((h * bound_w + w / 2) / w).max(1))
    } else {
        (((w * bound_h + h / 2) / h).max(1), bound_h)
    };
    (
        u32::try_from(nw).unwrap_or(u32::MAX),
        u32::try_from(nh).unwrap_or(u32::MAX),
    )
}

/// FNV-1a over the source bytes, used as the media identity.
#[must_use]
pub fn fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

fn frame_bytes(width: u32, height: u32) -> Result<usize, MediaError> {
    usize::try_from(width)
        .ok()
        .and_then(|w| w.checked_mul(usize::try_from(height).ok()?))
        .and_then(|px| px.checked_mul(4))
        .ok_or(MediaError::OverBudget)
}

/// Decode `request.bytes`, honoring every ceiling and the cancellation generation.
///
/// # Errors
/// [`MediaError`] describing the first violated limit or damaged input.
pub fn decode(request: &DecodeRequest) -> Result<DecodedMedia, MediaError> {
    let managed;
    let bytes: &[u8] = match &request.source {
        Source::Bytes(bytes) => bytes.as_slice(),
        Source::Managed(store, id) => {
            managed = store.read(id)?;
            &managed
        }
    };
    if bytes.len() > MAX_SOURCE_BYTES {
        return Err(MediaError::SourceTooLarge);
    }
    let format = detect(bytes).ok_or(MediaError::Unsupported)?;
    let identity = fingerprint(bytes);
    match format {
        ImageFormat::Gif => decode_gif(request, bytes, identity),
        _ => decode_still(request, bytes, format, identity),
    }
}

fn limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_AXIS);
    limits.max_image_height = Some(MAX_AXIS);
    // One canvas, the previous-frame copy and the composed frame at the ceiling, plus slack.
    limits.max_alloc = Some(64 * 1024 * 1024);
    limits
}

fn scale_frame(
    request: &DecodeRequest,
    lease: &mut super::BudgetLease,
    image: &RgbaImage,
    (target_w, target_h): (u32, u32),
) -> Result<Arc<[u8]>, MediaError> {
    // Charge before allocating the retained frame.
    lease.charge(frame_bytes(target_w, target_h)?)?;
    let rgba = if (image.width(), image.height()) == (target_w, target_h) {
        image.as_raw().clone()
    } else {
        resize(image, target_w, target_h, FilterType::Triangle).into_raw()
    };
    if !request.is_current() {
        return Err(MediaError::Cancelled);
    }
    Ok(Arc::from(rgba))
}

fn decode_still(
    request: &DecodeRequest,
    bytes: &[u8],
    format: ImageFormat,
    identity: u64,
) -> Result<DecodedMedia, MediaError> {
    // Dimensions are checked before any limit is armed, so an oversized canvas is reported as such.
    let reader = image::ImageReader::with_format(Cursor::new(bytes), format);
    let mut decoder = reader.into_decoder().map_err(|_| MediaError::Corrupt)?;
    let (width, height) = decoder.dimensions();
    if width > MAX_AXIS || height > MAX_AXIS {
        return Err(MediaError::DimensionsTooLarge);
    }
    if width == 0 || height == 0 {
        return Err(MediaError::Corrupt);
    }
    decoder
        .set_limits(limits())
        .map_err(|_| MediaError::DimensionsTooLarge)?;
    let image = DynamicImage::from_decoder(decoder)
        .map_err(|_| MediaError::Corrupt)?
        .into_rgba8();
    if !request.is_current() {
        return Err(MediaError::Cancelled);
    }
    let target = target_size(width, height, request.display, request.quality);
    let mut lease = request.budget.lease();
    let rgba = scale_frame(request, &mut lease, &image, target)?;
    Ok(DecodedMedia::new(
        target.0,
        target.1,
        vec![MediaFrame {
            rgba,
            delay_ms: REPLACEMENT_DELAY_MS,
        }],
        Repeat::Plays(1),
        identity,
        lease,
    ))
}

/// What a cheap pass over the GIF block structure finds before any frame is decoded.
#[derive(Debug, PartialEq, Eq)]
struct GifInfo {
    width: u32,
    height: u32,
    frames: usize,
    /// The NETSCAPE loop count: `Some(0)` loops forever, `None` plays once.
    loops: Option<u16>,
}

fn u16_at(bytes: &[u8], at: usize) -> Result<u32, MediaError> {
    let pair = bytes.get(at..at + 2).ok_or(MediaError::Corrupt)?;
    Ok(u32::from(u16::from_le_bytes([pair[0], pair[1]])))
}

fn skip_sub_blocks(bytes: &[u8], mut at: usize) -> Result<usize, MediaError> {
    loop {
        let len = usize::from(*bytes.get(at).ok_or(MediaError::Corrupt)?);
        at += 1;
        if len == 0 {
            return Ok(at);
        }
        at = at.checked_add(len).ok_or(MediaError::Corrupt)?;
    }
}

fn scan_gif(bytes: &[u8]) -> Result<GifInfo, MediaError> {
    let width = u16_at(bytes, 6)?;
    let height = u16_at(bytes, 8)?;
    let packed = *bytes.get(10).ok_or(MediaError::Corrupt)?;
    if width > MAX_AXIS || height > MAX_AXIS {
        return Err(MediaError::DimensionsTooLarge);
    }
    if width == 0 || height == 0 {
        return Err(MediaError::Corrupt);
    }
    let mut at = 13;
    if packed & 0x80 != 0 {
        at += 3 << ((packed & 7) + 1);
    }
    let mut info = GifInfo {
        width,
        height,
        frames: 0,
        loops: None,
    };
    loop {
        match *bytes.get(at).ok_or(MediaError::Corrupt)? {
            0x3B => return Ok(info),
            0x21 => {
                let label = *bytes.get(at + 1).ok_or(MediaError::Corrupt)?;
                at += 2;
                if label == 0xFF {
                    let len = usize::from(*bytes.get(at).ok_or(MediaError::Corrupt)?);
                    let id = bytes.get(at + 1..at + 1 + len).ok_or(MediaError::Corrupt)?;
                    if id == b"NETSCAPE2.0" || id == b"ANIMEXTS1.0" {
                        let data = at + 1 + len;
                        if bytes.get(data) == Some(&3) && bytes.get(data + 1) == Some(&1) {
                            let count = u16_at(bytes, data + 2)?;
                            info.loops = Some(u16::try_from(count).unwrap_or(u16::MAX));
                        }
                    }
                }
                at = skip_sub_blocks(bytes, at)?;
            }
            0x2C => {
                info.frames += 1;
                if info.frames > MAX_FRAMES {
                    return Err(MediaError::TooManyFrames);
                }
                let (left, top) = (u16_at(bytes, at + 1)?, u16_at(bytes, at + 3)?);
                let (w, h) = (u16_at(bytes, at + 5)?, u16_at(bytes, at + 7)?);
                if w == 0 || h == 0 || left + w > width || top + h > height {
                    return Err(MediaError::Corrupt);
                }
                let local = *bytes.get(at + 9).ok_or(MediaError::Corrupt)?;
                at += 10;
                if local & 0x80 != 0 {
                    at += 3 << ((local & 7) + 1);
                }
                at += 1; // LZW minimum code size
                at = skip_sub_blocks(bytes, at)?;
            }
            _ => return Err(MediaError::Corrupt),
        }
    }
}

fn normalized_delay(numer: u32, denom: u32) -> u32 {
    let ms = numer.checked_div(denom).unwrap_or(0);
    if ms <= MIN_GIF_DELAY_MS {
        REPLACEMENT_DELAY_MS
    } else {
        ms
    }
}

fn decode_gif(
    request: &DecodeRequest,
    bytes: &[u8],
    identity: u64,
) -> Result<DecodedMedia, MediaError> {
    let info = scan_gif(bytes)?;
    if info.frames == 0 {
        return Err(MediaError::Corrupt);
    }
    let mut decoder = GifDecoder::new(Cursor::new(bytes)).map_err(|_| MediaError::Corrupt)?;
    decoder
        .set_limits(limits())
        .map_err(|_| MediaError::DimensionsTooLarge)?;
    let target = target_size(info.width, info.height, request.display, request.quality);
    let mut lease = request.budget.lease();
    let mut frames = Vec::with_capacity(info.frames);
    for frame in decoder.into_frames() {
        if !request.is_current() {
            return Err(MediaError::Cancelled);
        }
        if frames.len() >= MAX_FRAMES {
            return Err(MediaError::TooManyFrames);
        }
        let frame = frame.map_err(|_| MediaError::Corrupt)?;
        let (numer, denom) = frame.delay().numer_denom_ms();
        let delay_ms = normalized_delay(numer, denom);
        // The decoder has composited this frame's delta and disposal onto the full canvas.
        let buffer = frame.into_buffer();
        if (buffer.width(), buffer.height()) != (info.width, info.height) {
            return Err(MediaError::Corrupt);
        }
        let rgba = scale_frame(request, &mut lease, &buffer, target)?;
        frames.push(MediaFrame { rgba, delay_ms });
    }
    if frames.len() != info.frames {
        return Err(MediaError::Corrupt);
    }
    let repeat = match info.loops {
        None => Repeat::Plays(1),
        Some(0) => Repeat::Infinite,
        Some(n) => Repeat::Plays(u32::from(n) + 1),
    };
    Ok(DecodedMedia::new(
        target.0, target.1, frames, repeat, identity, lease,
    ))
}

#[cfg(test)]
mod tests {
    use image::codecs::png::PngEncoder;
    use image::{ExtendedColorType, ImageEncoder};

    use super::super::MediaBudget;
    use super::super::test_gif::{TestFrame, gif};
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let pixels = vec![200_u8; usize::try_from(width * height * 4).unwrap()];
        let mut out = Vec::new();
        PngEncoder::new(&mut out)
            .write_image(&pixels, width, height, ExtendedColorType::Rgba8)
            .unwrap();
        out
    }

    fn request(bytes: Vec<u8>) -> DecodeRequest {
        DecodeRequest::standalone(
            Arc::new(bytes),
            None,
            Quality::Auto,
            MediaBudget::standard(),
        )
    }

    fn pixel(media: &DecodedMedia, frame: usize, x: u32, y: u32) -> [u8; 4] {
        let at = usize::try_from((y * media.width + x) * 4).unwrap();
        let p = &media.frames[frame].rgba[at..at + 4];
        [p[0], p[1], p[2], p[3]]
    }

    const RED: [u8; 4] = [255, 0, 0, 255];
    const GREEN: [u8; 4] = [0, 255, 0, 255];

    #[test]
    fn magic_detection_accepts_only_png_jpeg_and_gif() {
        assert_eq!(detect(&png(2, 2)), Some(ImageFormat::Png));
        assert_eq!(detect(&[0xFF, 0xD8, 0xFF, 0xE0]), Some(ImageFormat::Jpeg));
        assert_eq!(detect(b"GIF89a\x01\x00"), Some(ImageFormat::Gif));
        assert_eq!(detect(b"GIF87a"), Some(ImageFormat::Gif));
        for bad in [
            &b""[..],
            b"BM\x00\x00",
            b"RIFF\x00\x00\x00\x00WEBPVP8 ",
            b"<svg xmlns='http://www.w3.org/2000/svg'/>",
            b"GIF90a",
            b"\x89PNG",
        ] {
            assert_eq!(detect(bad), None, "{bad:?}");
            assert_eq!(
                decode(&request(bad.to_vec())).unwrap_err(),
                MediaError::Unsupported
            );
        }
    }

    #[test]
    fn a_still_image_decodes_to_one_frame_without_upscaling() {
        let media = decode(&request(png(50, 30))).unwrap();
        assert_eq!((media.width, media.height), (50, 30));
        assert_eq!(media.frames.len(), 1);
        assert!(!media.is_animated());
        assert_eq!(media.repeat, Repeat::Plays(1));
        assert_eq!(media.retained_bytes(), 50 * 30 * 4);
        assert_eq!(media.frames[0].rgba.len(), 50 * 30 * 4);
    }

    #[test]
    fn target_size_fits_inside_the_smaller_of_display_and_quality_cap() {
        let ts = target_size;
        // No upscaling.
        assert_eq!(ts(50, 50, None, Quality::Auto), (50, 50));
        assert_eq!(ts(1920, 1080, None, Quality::Auto), (1920, 1080));
        // Fallback 1920x1080 bound, aspect kept.
        assert_eq!(ts(2000, 1000, None, Quality::Auto), (1920, 960));
        assert_eq!(ts(1000, 2000, None, Quality::Auto), (540, 1080));
        // The display extent shrinks the bound; the cap shrinks it further.
        assert_eq!(ts(400, 200, Some((100, 100)), Quality::Auto), (100, 50));
        assert_eq!(
            ts(2000, 1000, Some((3840, 2160)), Quality::Auto),
            (1920, 960)
        );
        assert_eq!(ts(2000, 1000, Some((3840, 2160)), Quality::Low), (960, 480));
        assert_eq!(
            ts(2048, 2048, Some((4096, 4096)), Quality::High),
            (2048, 2048)
        );
        assert_eq!(
            ts(2048, 2048, Some((4096, 4096)), Quality::Auto),
            (1080, 1080)
        );
        assert_eq!(ts(10, 1, Some((1, 1)), Quality::Auto), (1, 1));
        assert_eq!(ts(4000, 1, None, Quality::Auto), (1920, 1));
    }

    #[test]
    fn large_stills_are_downscaled_to_the_display_extent() {
        let mut req = request(png(400, 200));
        req.display = Some((100, 100));
        let media = decode(&req).unwrap();
        assert_eq!((media.width, media.height), (100, 50));
        assert_eq!(media.retained_bytes(), 100 * 50 * 4);
    }

    #[test]
    fn dimension_ceiling_is_enforced_before_decoding() {
        assert!(decode(&request(png(2048, 1))).is_ok());
        assert_eq!(
            decode(&request(png(2049, 1))).unwrap_err(),
            MediaError::DimensionsTooLarge
        );
        assert_eq!(
            decode(&request(png(1, 2049))).unwrap_err(),
            MediaError::DimensionsTooLarge
        );
        let wide = gif(2049, 1, None, &[]);
        assert_eq!(
            decode(&request(wide)).unwrap_err(),
            MediaError::DimensionsTooLarge
        );
    }

    #[test]
    fn source_size_ceiling_is_16_mib() {
        let mut huge = png(2, 2);
        huge.resize(MAX_SOURCE_BYTES + 1, 0);
        assert_eq!(
            decode(&request(huge)).unwrap_err(),
            MediaError::SourceTooLarge
        );
    }

    #[test]
    fn truncated_and_corrupt_inputs_are_rejected() {
        let whole = png(40, 40);
        let truncated = whole[..whole.len() / 2].to_vec();
        assert_eq!(
            decode(&request(truncated)).unwrap_err(),
            MediaError::Corrupt
        );
        let frames = [TestFrame::full(4, 4, 0, 10), TestFrame::full(4, 4, 1, 10)];
        let good = gif(4, 4, None, &frames);
        assert!(decode(&request(good.clone())).is_ok());
        assert_eq!(
            decode(&request(good[..good.len() - 12].to_vec())).unwrap_err(),
            MediaError::Corrupt,
            "a GIF that ends before its trailer"
        );
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        jpeg.extend_from_slice(b"JFIF\0garbage garbage garbage");
        assert_eq!(decode(&request(jpeg)).unwrap_err(), MediaError::Corrupt);
        // A frame rectangle outside the canvas.
        let outside = gif(
            4,
            4,
            None,
            &[TestFrame {
                left: 3,
                ..TestFrame::full(2, 2, 0, 10)
            }],
        );
        assert_eq!(decode(&request(outside)).unwrap_err(), MediaError::Corrupt);
        // No frames at all.
        assert_eq!(
            decode(&request(gif(4, 4, None, &[]))).unwrap_err(),
            MediaError::Corrupt
        );
    }

    #[test]
    fn gif_deltas_transparency_and_disposal_are_composited() {
        // Frame 1 paints the whole canvas red and keeps it. Frame 2 draws a 2x2 green block at
        // (1,1) whose bottom-right pixel is transparent, so red shows through, then disposes to the
        // background. Frame 3 draws a 1x1 green pixel at (3,3): the disposed block is transparent.
        let frames = [
            TestFrame::full(4, 4, 0, 10),
            TestFrame {
                left: 1,
                top: 1,
                width: 2,
                height: 2,
                pixels: vec![1, 1, 1, 3],
                delay_cs: 20,
                dispose: 2,
                transparent: Some(3),
            },
            TestFrame {
                left: 3,
                top: 3,
                width: 1,
                height: 1,
                pixels: vec![1],
                delay_cs: 20,
                dispose: 1,
                transparent: None,
            },
        ];
        let media = decode(&request(gif(4, 4, None, &frames))).unwrap();
        assert_eq!((media.width, media.height), (4, 4));
        assert_eq!(media.frames.len(), 3);
        assert_eq!(pixel(&media, 0, 0, 0), RED);
        assert_eq!(pixel(&media, 1, 1, 1), GREEN);
        assert_eq!(
            pixel(&media, 1, 2, 2),
            RED,
            "transparent delta shows the prior frame"
        );
        assert_eq!(
            pixel(&media, 1, 0, 0),
            RED,
            "outside the delta is unchanged"
        );
        assert_eq!(pixel(&media, 2, 3, 3), GREEN);
        assert_eq!(
            pixel(&media, 2, 1, 1)[3],
            0,
            "restore-to-background clears the block"
        );
        assert_eq!(pixel(&media, 2, 0, 0), RED);
    }

    #[test]
    fn gif_delays_are_normalized_and_loops_follow_the_metadata() {
        let frames: Vec<_> = [0_u16, 1, 2, 5, 150]
            .into_iter()
            .map(|cs| TestFrame::full(2, 2, 0, cs))
            .collect();
        let media = decode(&request(gif(2, 2, None, &frames))).unwrap();
        let delays: Vec<_> = media.frames.iter().map(|f| f.delay_ms).collect();
        assert_eq!(
            delays,
            [100, 100, 20, 50, 1500],
            "10 ms or less becomes 100 ms"
        );
        assert_eq!(
            media.repeat,
            Repeat::Plays(1),
            "absent metadata is one play-through"
        );
        let looped = |count| {
            decode(&request(gif(2, 2, Some(count), &frames)))
                .unwrap()
                .repeat
        };
        assert_eq!(looped(0), Repeat::Infinite);
        assert_eq!(
            looped(1),
            Repeat::Plays(2),
            "finite repeats follow the first play"
        );
        assert_eq!(looped(3), Repeat::Plays(4));
    }

    #[test]
    fn frame_count_ceiling_is_300() {
        let frame = |i: u8| TestFrame::full(1, 1, i % 4, 5);
        let ok: Vec<_> = (0..300)
            .map(|i| frame(u8::try_from(i % 4).unwrap()))
            .collect();
        assert_eq!(
            decode(&request(gif(1, 1, None, &ok))).unwrap().frames.len(),
            300
        );
        let over: Vec<_> = (0..301)
            .map(|i| frame(u8::try_from(i % 4).unwrap()))
            .collect();
        assert_eq!(
            decode(&request(gif(1, 1, None, &over))).unwrap_err(),
            MediaError::TooManyFrames
        );
    }

    #[test]
    fn aggregate_memory_is_charged_before_retention_and_released() {
        // 64x64 frames are 16 KiB each; 100 of them need 1.6 MB.
        let frames: Vec<_> = (0..100).map(|_| TestFrame::full(64, 64, 0, 5)).collect();
        let bytes = Arc::new(gif(64, 64, None, &frames));
        let tight = MediaBudget::new(1024 * 1024);
        let mut req = DecodeRequest::standalone(bytes.clone(), None, Quality::Auto, tight.clone());
        assert_eq!(decode(&req).unwrap_err(), MediaError::OverBudget);
        assert_eq!(tight.used(), 0, "a rejected decode retains nothing");

        // The cap is shared by active and preview: the second decode does not fit.
        let shared = MediaBudget::new(2 * 64 * 64 * 4 * 100 - 1);
        req.budget = shared.clone();
        let first = decode(&req).unwrap();
        assert_eq!(shared.used(), 64 * 64 * 4 * 100);
        assert_eq!(decode(&req).unwrap_err(), MediaError::OverBudget);
        assert_eq!(shared.used(), 64 * 64 * 4 * 100);
        drop(first);
        assert_eq!(shared.used(), 0);
        assert!(decode(&req).is_ok());
    }

    #[test]
    fn a_superseded_generation_cancels_between_frames() {
        let frames: Vec<_> = (0..10).map(|_| TestFrame::full(8, 8, 0, 5)).collect();
        let mut req = request(gif(8, 8, None, &frames));
        req.generation = 7; // the shared `latest` still says 1
        assert_eq!(decode(&req).unwrap_err(), MediaError::Cancelled);
    }

    #[test]
    fn identity_is_stable_for_the_same_bytes() {
        let a = decode(&request(png(8, 8))).unwrap();
        let b = decode(&request(png(8, 8))).unwrap();
        let c = decode(&request(png(9, 8))).unwrap();
        assert_eq!(a.identity, b.identity);
        assert_ne!(a.identity, c.identity);
    }
}
