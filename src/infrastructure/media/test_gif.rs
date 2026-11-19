//! A tiny `GIF89a` writer for tests. It uses "stored" LZW (a clear code every two pixels) so frames
//! with arbitrary rectangles, disposal, transparency and loop metadata can be built exactly.

/// A four-color global palette: red, green, blue, white.
const PALETTE: [[u8; 3]; 4] = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 255]];

#[derive(Debug, Clone)]
pub struct TestFrame {
    pub left: u32,
    pub top: u32,
    pub width: u32,
    pub height: u32,
    /// Palette indices, row-major, `width * height` long.
    pub pixels: Vec<u8>,
    pub delay_cs: u16,
    /// GIF disposal method: 1 keep, 2 restore to background, 3 restore previous.
    pub dispose: u8,
    pub transparent: Option<u8>,
}

impl TestFrame {
    /// A frame covering `width` by `height` from the origin in one palette color.
    pub fn full(width: u32, height: u32, color: u8, delay_cs: u16) -> Self {
        Self {
            left: 0,
            top: 0,
            width,
            height,
            pixels: vec![color; usize::try_from(width * height).unwrap()],
            delay_cs,
            dispose: 1,
            transparent: None,
        }
    }
}

struct Bits {
    out: Vec<u8>,
    acc: u32,
    n: u32,
}

impl Bits {
    fn put(&mut self, code: u32, width: u32) {
        self.acc |= code << self.n;
        self.n += width;
        while self.n >= 8 {
            self.out.push((self.acc & 0xFF) as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.out.push((self.acc & 0xFF) as u8);
        }
        self.out
    }
}

fn lzw(pixels: &[u8]) -> Vec<u8> {
    const CLEAR: u32 = 4;
    const END: u32 = 5;
    let mut bits = Bits {
        out: Vec::new(),
        acc: 0,
        n: 0,
    };
    for pair in pixels.chunks(2) {
        bits.put(CLEAR, 3);
        for &p in pair {
            bits.put(u32::from(p), 3);
        }
    }
    bits.put(END, 3);
    bits.finish()
}

fn sub_blocks(out: &mut Vec<u8>, data: &[u8]) {
    for chunk in data.chunks(255) {
        out.push(u8::try_from(chunk.len()).unwrap());
        out.extend_from_slice(chunk);
    }
    out.push(0);
}

fn le(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&u16::try_from(value).unwrap().to_le_bytes());
}

/// A GIF with a `width` by `height` canvas. `loops` is the NETSCAPE loop count when present.
pub fn gif(width: u32, height: u32, loops: Option<u16>, frames: &[TestFrame]) -> Vec<u8> {
    let mut out = b"GIF89a".to_vec();
    le(&mut out, width);
    le(&mut out, height);
    out.extend_from_slice(&[0x81, 0, 0]); // global table of 4 colors, background index 0
    for color in PALETTE {
        out.extend_from_slice(&color);
    }
    if let Some(count) = loops {
        out.extend_from_slice(&[0x21, 0xFF, 0x0B]);
        out.extend_from_slice(b"NETSCAPE2.0");
        out.extend_from_slice(&[0x03, 0x01]);
        out.extend_from_slice(&count.to_le_bytes());
        out.push(0);
    }
    for frame in frames {
        out.extend_from_slice(&[0x21, 0xF9, 0x04]);
        out.push((frame.dispose << 2) | u8::from(frame.transparent.is_some()));
        out.extend_from_slice(&frame.delay_cs.to_le_bytes());
        out.push(frame.transparent.unwrap_or(0));
        out.push(0);
        out.push(0x2C);
        le(&mut out, frame.left);
        le(&mut out, frame.top);
        le(&mut out, frame.width);
        le(&mut out, frame.height);
        out.push(0);
        out.push(2); // LZW minimum code size
        sub_blocks(&mut out, &lzw(&frame.pixels));
    }
    out.push(0x3B);
    out
}
