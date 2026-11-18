use std::sync::Arc;

use eframe::egui::{self, Color32, ColorImage, TextureHandle, TextureOptions};

pub const WALLPAPER_WIDTH: usize = 1920;
pub const WALLPAPER_HEIGHT: usize = 1080;

#[derive(Debug, Clone)]
pub struct WallpaperSource {
    pub size_px: [u32; 2],
    pub generation: u64,
    pub color_image: Arc<ColorImage>,
    pub rgba_bytes: Arc<Vec<u8>>,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct EguiTextureStats {
    pub allocations: u64,
    pub partial_updates: u64,
}

pub struct EguiWallpaperTexture {
    handle: Option<TextureHandle>,
    uploaded_generation: u64,
    uploaded_size: [u32; 2],
    pub stats: EguiTextureStats,
}

impl EguiWallpaperTexture {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            handle: None,
            uploaded_generation: 0,
            uploaded_size: [0, 0],
            stats: EguiTextureStats {
                allocations: 0,
                partial_updates: 0,
            },
        }
    }

    pub fn ensure_uploaded(
        &mut self,
        ctx: &egui::Context,
        source: &WallpaperSource,
    ) -> egui::TextureId {
        match &mut self.handle {
            None => {
                let handle = ctx.load_texture(
                    "athena_glass_spike_wallpaper",
                    egui::ImageData::Color(Arc::clone(&source.color_image)),
                    TextureOptions::LINEAR,
                );
                self.uploaded_generation = source.generation;
                self.uploaded_size = source.size_px;
                self.stats.allocations += 1;
                let id = handle.id();
                self.handle = Some(handle);
                id
            }
            Some(handle) if self.uploaded_size != source.size_px => {
                let new_handle = ctx.load_texture(
                    "athena_glass_spike_wallpaper",
                    egui::ImageData::Color(Arc::clone(&source.color_image)),
                    TextureOptions::LINEAR,
                );
                self.uploaded_generation = source.generation;
                self.uploaded_size = source.size_px;
                self.stats.allocations += 1;
                let id = new_handle.id();
                *handle = new_handle;
                id
            }
            Some(handle) => {
                if self.uploaded_generation != source.generation {
                    handle.set_partial(
                        [0, 0],
                        egui::ImageData::Color(Arc::clone(&source.color_image)),
                        TextureOptions::LINEAR,
                    );
                    self.uploaded_generation = source.generation;
                    self.stats.partial_updates += 1;
                }
                handle.id()
            }
        }
    }
}

/// Generates a deterministic 16:9 bright/dark checker-gradient wallpaper.
///
/// Even variants produce high-contrast cool-sky vs deep-navy checkers with a diagonal gradient;
/// odd variants swap to warm-amber vs deep-plum checkers so `set_partial` and cache invalidation
/// can be verified visually and via counters.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
pub fn generate_wallpaper(generation: u64) -> WallpaperSource {
    let width = WALLPAPER_WIDTH;
    let height = WALLPAPER_HEIGHT;
    let total = width * height;
    let mut pixels = Vec::with_capacity(total);
    let mut rgba_bytes = Vec::with_capacity(total * 4);
    let alt_palette = generation % 2 == 1;
    let cell_px = 96usize;

    for y in 0..height {
        let ny = y as f32 / (height - 1) as f32;
        let cy = y / cell_px;
        let grid_line_y = y % cell_px < 2;
        for x in 0..width {
            let nx = x as f32 / (width - 1) as f32;
            let cx = x / cell_px;
            let is_bright = (cx + cy).is_multiple_of(2);
            let grid_line = grid_line_y || (x % cell_px < 2);

            let (red, green, blue) = if grid_line && (cx.is_multiple_of(4) || cy.is_multiple_of(4))
            {
                if alt_palette {
                    (255, 138, 101)
                } else {
                    (108, 140, 255)
                }
            } else if is_bright {
                if alt_palette {
                    let red = (242.0 + 13.0 * nx).round() as u8;
                    let green = (212.0 + 36.0 * (1.0 - ny)).round() as u8;
                    let blue = (176.0 + 62.0 * nx).round() as u8;
                    (red, green, blue)
                } else {
                    let red = (220.0 + 34.0 * nx).round() as u8;
                    let green = (234.0 + 20.0 * (1.0 - ny)).round() as u8;
                    let blue = (248.0 - 30.0 * ny + 7.0 * nx).round() as u8;
                    (red, green, blue)
                }
            } else if alt_palette {
                let red = (26.0 + 34.0 * nx).round() as u8;
                let green = (14.0 + 22.0 * ny).round() as u8;
                let blue = (34.0 + 38.0 * (1.0 - nx)).round() as u8;
                (red, green, blue)
            } else {
                let red = (10.0 + 28.0 * nx).round() as u8;
                let green = (14.0 + 26.0 * ny).round() as u8;
                let blue = (28.0 + 44.0 * (1.0 - ny)).round() as u8;
                (red, green, blue)
            };

            pixels.push(Color32::from_rgba_unmultiplied(red, green, blue, 255));
            rgba_bytes.extend_from_slice(&[red, green, blue, 255]);
        }
    }

    let color_image = Arc::new(ColorImage::new([width, height], pixels));
    WallpaperSource {
        size_px: [width as u32, height as u32],
        generation,
        color_image,
        rgba_bytes: Arc::new(rgba_bytes),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_wallpaper_pixels_match_raw_rgba_bytes() {
        let wp = generate_wallpaper(1);
        assert_eq!(wp.size_px, [1920, 1080]);
        assert_eq!(wp.color_image.size, [1920, 1080]);
        assert_eq!(wp.rgba_bytes.len(), 1920 * 1080 * 4);
        for (idx, px) in wp.color_image.pixels.iter().enumerate() {
            let base = idx * 4;
            assert_eq!(
                [px.r(), px.g(), px.b(), px.a()],
                [
                    wp.rgba_bytes[base],
                    wp.rgba_bytes[base + 1],
                    wp.rgba_bytes[base + 2],
                    wp.rgba_bytes[base + 3]
                ]
            );
        }
    }

    #[test]
    fn wallpaper_generations_share_dimensions_and_differ_in_content() {
        let wp1 = generate_wallpaper(1);
        let wp2 = generate_wallpaper(2);
        assert_eq!(wp1.size_px, wp2.size_px);
        assert_ne!(wp1.rgba_bytes, wp2.rgba_bytes);
    }
}
