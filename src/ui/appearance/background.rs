//! The wallpaper compositor (contract §14.2): Fit/Fill geometry, the minimum scrim that keeps text
//! legible over any wallpaper, and the non-interactive painter behind the shell chrome.

use eframe::egui::{Color32, CornerRadius, Painter, Rect, TextureId, Vec2, pos2};

use crate::domain::appearance::Fit;
use crate::ui::theme::effective::Material;
use crate::ui::theme::{Palette, composite, contrast_ratio};

/// What the shell paints behind its chrome this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Backdrop {
    /// The wallpaper texture and its pixel size; `None` paints only the solid canvas.
    pub texture: Option<(TextureId, Vec2)>,
    pub fit: Fit,
    /// Scrim opacity over the whole viewport, already clamped to the legibility floor.
    pub scrim: u8,
    pub material: Material,
}

impl Backdrop {
    /// The plain canvas: no wallpaper, opaque chrome.
    #[must_use]
    pub fn solid() -> Self {
        Self {
            texture: None,
            fit: Fit::Fill,
            scrim: 0,
            material: Material::Solid,
        }
    }

    #[must_use]
    pub fn has_wallpaper(&self) -> bool {
        self.texture.is_some()
    }
}

/// Where the image lands inside `viewport` and which part of it is sampled, both for the shell and
/// for any preview. Fit letterboxes the whole image; Fill centers an aspect-preserving cover crop.
#[must_use]
pub fn geometry(viewport: Rect, image: Vec2, fit: Fit) -> (Rect, Rect) {
    let full = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    if image.x <= 0.0 || image.y <= 0.0 || viewport.width() <= 0.0 || viewport.height() <= 0.0 {
        return (viewport, full);
    }
    let image_aspect = image.x / image.y;
    let view_aspect = viewport.width() / viewport.height();
    match fit {
        Fit::Fit => {
            let size = if image_aspect > view_aspect {
                Vec2::new(viewport.width(), viewport.width() / image_aspect)
            } else {
                Vec2::new(viewport.height() * image_aspect, viewport.height())
            };
            (Rect::from_center_size(viewport.center(), size), full)
        }
        Fit::Fill => {
            let uv = if image_aspect > view_aspect {
                let visible = view_aspect / image_aspect;
                Rect::from_min_max(
                    pos2((1.0 - visible) / 2.0, 0.0),
                    pos2(f32::midpoint(1.0, visible), 1.0),
                )
            } else {
                let visible = image_aspect / view_aspect;
                Rect::from_min_max(
                    pos2(0.0, (1.0 - visible) / 2.0),
                    pos2(1.0, f32::midpoint(1.0, visible)),
                )
            };
            (viewport, uv)
        }
    }
}

/// Every foreground drawn straight over the wallpaper, with the contrast it needs.
fn foregrounds(p: &Palette) -> [(Color32, f32); 5] {
    [
        (p.text, 4.5),
        (p.muted, 4.5),
        (p.accent, 4.5),
        (p.focus, 3.0),
        (p.text, 3.0),
    ]
}

fn legible_over(p: &Palette, alpha: u8, wallpaper: Color32) -> bool {
    let surface = composite(p.bg, alpha, wallpaper);
    foregrounds(p)
        .iter()
        .all(|&(fg, needed)| contrast_ratio(fg, surface) >= needed)
}

/// The smallest scrim opacity (in 8-bit steps) that keeps ordinary, muted and accent text at
/// 4.5:1 and focus rings and essential icons at 3:1 over both pure black and pure white wallpaper.
#[must_use]
pub fn min_scrim_alpha(p: &Palette) -> u8 {
    (0..=u8::MAX)
        .find(|&a| legible_over(p, a, Color32::BLACK) && legible_over(p, a, Color32::WHITE))
        .unwrap_or(u8::MAX)
}

/// The scrim opacity actually drawn: the requested dim (0–100 percent) raised to the floor.
#[must_use]
pub fn scrim_alpha(p: &Palette, dim: u8) -> u8 {
    let requested = (u16::from(dim.min(100)) * 255 + 50) / 100;
    u8::try_from(requested)
        .unwrap_or(u8::MAX)
        .max(min_scrim_alpha(p))
}

/// The lowest dim percentage that is not raised by the floor, for the Dim control's minimum.
#[must_use]
pub fn min_dim_percent(p: &Palette) -> u8 {
    let floor = u16::from(min_scrim_alpha(p));
    u8::try_from((floor * 100).div_ceil(255)).unwrap_or(100)
}

/// The sidebar fill opacity over the scrimmed wallpaper: the most translucent step at which every
/// nav state stays at its own contrast over both extremes, else fully solid.
#[must_use]
pub fn chrome_alpha(p: &Palette, scrim: u8) -> u8 {
    const STEPS: [u8; 4] = [168, 196, 224, 255];
    let selected = |surface: Color32| composite(p.accent, 36, surface);
    let ok = |alpha: u8, wallpaper: Color32| {
        let canvas = composite(p.bg, scrim, wallpaper);
        let panel = composite(p.panel, alpha, canvas);
        contrast_ratio(p.text, panel) >= 4.5
            && contrast_ratio(p.muted, panel) >= 4.5
            && contrast_ratio(p.accent, selected(panel)) >= 4.5
            && contrast_ratio(p.text, composite(p.card_hover, alpha, canvas)) >= 4.5
            && contrast_ratio(p.focus, panel) >= 3.0
    };
    STEPS
        .into_iter()
        .find(|&a| ok(a, Color32::BLACK) && ok(a, Color32::WHITE))
        .unwrap_or(u8::MAX)
}

fn scrim_color(p: &Palette, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(p.bg.r(), p.bg.g(), p.bg.b(), alpha)
}

/// Paint the canvas, the wallpaper and the scrim into `rect`. Purely decorative: it allocates no
/// widget and senses nothing, so pointer and scroll input pass straight through.
pub fn paint(painter: &Painter, rect: Rect, p: &Palette, backdrop: &Backdrop) {
    painter.rect_filled(rect, CornerRadius::ZERO, p.bg);
    let Some((texture, size)) = backdrop.texture else {
        return;
    };
    let (dest, uv) = geometry(rect, size, backdrop.fit);
    painter.image(texture, dest, uv, Color32::WHITE);
    if backdrop.scrim > 0 {
        painter.rect_filled(rect, CornerRadius::ZERO, scrim_color(p, backdrop.scrim));
    }
}

/// The color of one wallpaper pixel after the scrim, as the compositor produces it.
#[must_use]
pub fn composed_pixel(p: &Palette, scrim: u8, wallpaper: Color32) -> Color32 {
    composite(p.bg, scrim, wallpaper)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::settings::Theme;
    use crate::ui::theme::stock_palette;

    fn rect(w: f32, h: f32) -> Rect {
        Rect::from_min_size(pos2(10.0, 20.0), Vec2::new(w, h))
    }

    #[test]
    fn fit_letterboxes_the_whole_image() {
        // A wide image in a tall viewport: bars above and below, full-width.
        let (dest, uv) = geometry(rect(100.0, 100.0), Vec2::new(200.0, 100.0), Fit::Fit);
        assert!((dest.width() - 100.0).abs() < 1e-4 && (dest.height() - 50.0).abs() < 1e-4);
        assert!((dest.center() - rect(100.0, 100.0).center()).length() < 1e-4);
        assert_eq!(uv, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)));
        // A tall image in a wide viewport: bars at the sides.
        let (dest, _) = geometry(rect(200.0, 100.0), Vec2::new(100.0, 200.0), Fit::Fit);
        assert!((dest.width() - 50.0).abs() < 1e-4 && (dest.height() - 100.0).abs() < 1e-4);
        // Equal aspect fills exactly.
        let (dest, _) = geometry(rect(160.0, 90.0), Vec2::new(1920.0, 1080.0), Fit::Fit);
        assert!((dest.width() - 160.0).abs() < 1e-3 && (dest.height() - 90.0).abs() < 1e-3);
    }

    #[test]
    fn fill_centers_an_aspect_cover_crop() {
        let view = rect(100.0, 100.0);
        let (dest, uv) = geometry(view, Vec2::new(200.0, 100.0), Fit::Fill);
        assert_eq!(dest, view);
        assert!((uv.min.x - 0.25).abs() < 1e-5 && (uv.max.x - 0.75).abs() < 1e-5);
        assert!(uv.min.y.abs() < 1e-6 && (uv.max.y - 1.0).abs() < 1e-6);
        let (_, uv) = geometry(view, Vec2::new(100.0, 200.0), Fit::Fill);
        assert!((uv.min.y - 0.25).abs() < 1e-5 && (uv.max.y - 0.75).abs() < 1e-5);
        // Degenerate sizes fall back to the whole viewport.
        let (dest, uv) = geometry(view, Vec2::ZERO, Fit::Fill);
        assert_eq!(dest, view);
        assert_eq!(uv, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)));
    }

    #[test]
    fn the_scrim_floor_is_minimal_and_legible_over_black_and_white() {
        for theme in [Theme::Dark, Theme::Light] {
            let p = stock_palette(theme);
            let floor = min_scrim_alpha(&p);
            assert!(floor > 0 && floor < 255, "{theme:?} floor {floor}");
            for wallpaper in [Color32::BLACK, Color32::WHITE] {
                for a in floor..=u8::MAX {
                    assert!(legible_over(&p, a, wallpaper), "{theme:?} alpha {a}");
                }
            }
            // One step lower fails for at least one extreme.
            let lower = floor - 1;
            assert!(
                !(legible_over(&p, lower, Color32::BLACK)
                    && legible_over(&p, lower, Color32::WHITE)),
                "{theme:?} floor is not minimal"
            );
            // Direct check of every allowed foreground at the floor.
            for wallpaper in [Color32::BLACK, Color32::WHITE] {
                let surface = composed_pixel(&p, floor, wallpaper);
                assert!(contrast_ratio(p.text, surface) >= 4.5);
                assert!(contrast_ratio(p.muted, surface) >= 4.5);
                assert!(contrast_ratio(p.accent, surface) >= 4.5);
                assert!(contrast_ratio(p.focus, surface) >= 3.0);
            }
        }
    }

    #[test]
    fn saturated_and_mid_wallpapers_stay_legible_at_the_floor() {
        for theme in [Theme::Dark, Theme::Light] {
            let p = stock_palette(theme);
            let floor = min_scrim_alpha(&p);
            for wallpaper in [
                Color32::from_rgb(255, 0, 0),
                Color32::from_rgb(0, 255, 0),
                Color32::from_rgb(0, 0, 255),
                Color32::from_rgb(255, 255, 0),
                Color32::from_rgb(0, 255, 255),
                Color32::from_rgb(255, 0, 255),
                Color32::from_gray(128),
            ] {
                let surface = composed_pixel(&p, floor, wallpaper);
                for fg in [p.text, p.muted, p.accent] {
                    assert!(
                        contrast_ratio(fg, surface) >= 4.5,
                        "{theme:?} {wallpaper:?} over {surface:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn dim_is_clamped_up_to_the_floor_and_never_exceeds_opaque() {
        let p = stock_palette(Theme::Dark);
        let floor = min_scrim_alpha(&p);
        assert_eq!(scrim_alpha(&p, 0), floor);
        assert_eq!(scrim_alpha(&p, 100), 255);
        assert_eq!(
            scrim_alpha(&p, 250),
            255,
            "imported out-of-range dim is clamped"
        );
        let min_percent = min_dim_percent(&p);
        assert!(scrim_alpha(&p, min_percent) >= floor);
        assert_eq!(
            scrim_alpha(&p, 0),
            scrim_alpha(&p, min_percent.saturating_sub(1)).min(floor)
        );
        let mid = scrim_alpha(&p, 90);
        assert!(mid >= scrim_alpha(&p, 80));
    }

    #[test]
    fn translucent_chrome_keeps_every_nav_state_legible_or_goes_solid() {
        for theme in [Theme::Dark, Theme::Light] {
            let p = stock_palette(theme);
            let scrim = scrim_alpha(&p, 0);
            let alpha = chrome_alpha(&p, scrim);
            for wallpaper in [Color32::BLACK, Color32::WHITE, Color32::from_rgb(255, 0, 0)] {
                let canvas = composite(p.bg, scrim, wallpaper);
                let panel = composite(p.panel, alpha, canvas);
                assert!(contrast_ratio(p.text, panel) >= 4.5);
                assert!(contrast_ratio(p.muted, panel) >= 4.5);
                assert!(
                    contrast_ratio(p.accent, composite(p.accent, 36, panel)) >= 4.5,
                    "{theme:?} selected nav over {wallpaper:?}"
                );
            }
        }
        // A scrim far below the floor cannot be rescued by chrome alone: it goes fully opaque.
        let p = stock_palette(Theme::Dark);
        assert_eq!(chrome_alpha(&p, 0), 255);
    }

    #[test]
    fn solid_backdrop_has_no_wallpaper() {
        assert!(!Backdrop::solid().has_wallpaper());
    }
}
