//! Embedded icon set: Lucide 0.468.0 (ISC) 24-unit geometry painted as egui line shapes, so icons
//! never depend on system fonts, runtime SVG loading or image dependencies. See
//! `assets/icons/PROVENANCE.md` and `tools/icons/gen_icons.py`.

use eframe::egui::{
    self, Color32, Pos2, Rect, Response, Role, Sense, Shape, Stroke, Vec2, WidgetInfo,
};

use crate::domain::transaction::TransactionType;
use crate::ui::icon_paths::{self, Poly};
use crate::ui::theme::Palette;
use crate::ui::tokens;

/// One icon of the embedded set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Icon {
    List,
    CalendarDays,
    Trash2,
    Settings,
    ChevronLeft,
    ChevronRight,
    Plus,
    X,
    Pencil,
    /// Income.
    ArrowDownLeft,
    /// Expense.
    ArrowUpRight,
    /// Refund.
    Undo2,
    /// Settlement.
    ArrowLeftRight,
    /// Goals (1.4).
    Target,
    Search,
    /// A saved closing (1.4).
    Bookmark,
    /// A closing that changed or could not be checked (1.4).
    TriangleAlert,
}

impl Icon {
    pub const ALL: [Icon; 17] = [
        Icon::List,
        Icon::CalendarDays,
        Icon::Trash2,
        Icon::Settings,
        Icon::ChevronLeft,
        Icon::ChevronRight,
        Icon::Plus,
        Icon::X,
        Icon::Pencil,
        Icon::ArrowDownLeft,
        Icon::ArrowUpRight,
        Icon::Undo2,
        Icon::ArrowLeftRight,
        Icon::Target,
        Icon::Search,
        Icon::Bookmark,
        Icon::TriangleAlert,
    ];

    /// Upstream Lucide name, the provenance key.
    #[must_use]
    pub fn lucide_name(self) -> &'static str {
        match self {
            Icon::List => "list",
            Icon::CalendarDays => "calendar-days",
            Icon::Trash2 => "trash-2",
            Icon::Settings => "settings",
            Icon::ChevronLeft => "chevron-left",
            Icon::ChevronRight => "chevron-right",
            Icon::Plus => "plus",
            Icon::X => "x",
            Icon::Pencil => "pencil",
            Icon::ArrowDownLeft => "arrow-down-left",
            Icon::ArrowUpRight => "arrow-up-right",
            Icon::Undo2 => "undo-2",
            Icon::ArrowLeftRight => "arrow-left-right",
            Icon::Target => "target",
            Icon::Search => "search",
            Icon::Bookmark => "bookmark",
            Icon::TriangleAlert => "triangle-alert",
        }
    }

    /// 24-unit polylines.
    #[must_use]
    pub fn paths(self) -> &'static [Poly] {
        match self {
            Icon::List => icon_paths::LIST,
            Icon::CalendarDays => icon_paths::CALENDAR_DAYS,
            Icon::Trash2 => icon_paths::TRASH_2,
            Icon::Settings => icon_paths::SETTINGS,
            Icon::ChevronLeft => icon_paths::CHEVRON_LEFT,
            Icon::ChevronRight => icon_paths::CHEVRON_RIGHT,
            Icon::Plus => icon_paths::PLUS,
            Icon::X => icon_paths::X,
            Icon::Pencil => icon_paths::PENCIL,
            Icon::ArrowDownLeft => icon_paths::ARROW_DOWN_LEFT,
            Icon::ArrowUpRight => icon_paths::ARROW_UP_RIGHT,
            Icon::Undo2 => icon_paths::UNDO_2,
            Icon::ArrowLeftRight => icon_paths::ARROW_LEFT_RIGHT,
            Icon::Target => icon_paths::TARGET,
            Icon::Search => icon_paths::SEARCH,
            Icon::Bookmark => icon_paths::BOOKMARK,
            Icon::TriangleAlert => icon_paths::TRIANGLE_ALERT,
        }
    }

    /// Icon for a transaction kind: income down-left, expense up-right, refund undo,
    /// settlement left-right.
    #[must_use]
    pub fn for_kind(kind: TransactionType) -> Icon {
        match kind {
            TransactionType::Income => Icon::ArrowDownLeft,
            TransactionType::Expense => Icon::ArrowUpRight,
            TransactionType::Refund => Icon::Undo2,
            TransactionType::Settlement => Icon::ArrowLeftRight,
        }
    }
}

/// Stroke width of the set in 24-unit space.
const UNITS: f32 = 24.0;
const STROKE_UNITS: f32 = 2.0;

/// Paint `icon` scaled into `rect` (square-fitted, centered) with a 2-unit round stroke.
pub fn paint(ui: &egui::Ui, rect: Rect, icon: Icon, color: Color32) {
    paint_with(ui.painter(), rect, icon, color);
}

/// [`paint`] on an explicit painter.
pub fn paint_with(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let side = rect.width().min(rect.height());
    let scale = side / UNITS;
    let origin = rect.center() - Vec2::splat(side / 2.0);
    let stroke_w = STROKE_UNITS * scale;
    let stroke = Stroke::new(stroke_w, color);
    let map = |p: &[f32; 2]| Pos2::new(origin.x + p[0] * scale, origin.y + p[1] * scale);
    for poly in icon.paths() {
        let points: Vec<Pos2> = poly.iter().map(map).collect();
        let closed = poly.len() > 2 && poly.first() == poly.last();
        if !closed {
            // Round caps: egui line ends are butt, so cap open polylines with dots.
            for end in [poly.first(), poly.last()].into_iter().flatten() {
                painter.circle_filled(map(end), stroke_w / 2.0, color);
            }
        }
        painter.add(Shape::line(points, stroke));
    }
}

/// A square icon-only button with a localized accessible name. Returns the response; the name is
/// also the hover text.
pub fn button(ui: &mut egui::Ui, p: &Palette, icon: Icon, label: &str) -> Response {
    let size = Vec2::splat(tokens::control_height());
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| WidgetInfo::labeled(Role::Button, true, label));
    if ui.is_rect_visible(rect) {
        let fill = if response.is_pointer_button_down_on() {
            tokens::selected_fill(p, p.card)
        } else if response.hovered() {
            p.card_hover
        } else {
            p.card
        };
        let radius = egui::CornerRadius::same(tokens::CONTROL_RADIUS);
        let painter = ui.painter();
        painter.rect_filled(rect, radius, fill);
        painter.rect_stroke(
            rect,
            radius,
            Stroke::new(tokens::BORDER_WIDTH, p.border),
            egui::StrokeKind::Inside,
        );
        if response.has_focus() {
            painter.rect_stroke(
                rect,
                radius,
                Stroke::new(2.0, p.focus),
                egui::StrokeKind::Inside,
            );
        }
        let icon_rect = Rect::from_center_size(rect.center(), Vec2::splat(tokens::ICON_SIZE));
        paint_with(painter, icon_rect, icon, p.text);
    }
    response
        .on_hover_text(label)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::settings::{Language, Theme};
    use crate::i18n::strings;
    use crate::ui::theme::palette;

    #[test]
    fn embedded_icons_cover_ui_roles_without_font_glyphs() {
        // Every icon has geometry inside the 24-unit box, with finite points.
        for icon in Icon::ALL {
            let polys = icon.paths();
            assert!(!polys.is_empty(), "{icon:?} has no geometry");
            for poly in polys {
                assert!(poly.len() >= 2, "{icon:?} has a degenerate polyline");
                for [x, y] in *poly {
                    assert!(
                        x.is_finite()
                            && y.is_finite()
                            && (0.0..=24.0).contains(x)
                            && (0.0..=24.0).contains(y),
                        "{icon:?} point out of the 24-unit box: {x},{y}"
                    );
                }
            }
            assert!(!icon.lucide_name().is_empty());
        }
        // Distinct geometry per role.
        let roles = [
            Icon::List,
            Icon::CalendarDays,
            Icon::Trash2,
            Icon::Settings,
            Icon::ChevronLeft,
            Icon::ChevronRight,
            Icon::Plus,
            Icon::X,
            Icon::Pencil,
            Icon::for_kind(TransactionType::Income),
            Icon::for_kind(TransactionType::Expense),
            Icon::for_kind(TransactionType::Refund),
            Icon::for_kind(TransactionType::Settlement),
        ];
        for (i, a) in roles.iter().enumerate() {
            for b in &roles[i + 1..] {
                assert_ne!(a.paths(), b.paths(), "{a:?} and {b:?} share geometry");
            }
        }
        // Screens map to icons with no glyph string.
        for screen in crate::app::navigation::Screen::ALL {
            assert!(!screen.icon().paths().is_empty());
        }
    }

    #[test]
    fn every_icon_has_a_vendored_source_and_license() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/icons");
        let license = std::fs::read_to_string(root.join("LICENSE.txt")).unwrap();
        assert!(license.starts_with("ISC License"));
        for icon in Icon::ALL {
            let svg = std::fs::read_to_string(
                root.join("lucide")
                    .join(format!("{}.svg", icon.lucide_name())),
            )
            .unwrap();
            assert!(svg.contains("lucide-static v0.468.0"), "{icon:?}");
        }
    }

    #[test]
    fn icon_button_is_named_focusable_and_keyboard_activated() {
        use egui_kittest::kittest::Queryable;
        let p = palette(Theme::Dark);
        let label = strings(Language::PtBr).month_prev;
        let mut clicked = 0;
        let mut harness = egui_kittest::Harness::new_ui(|ui| {
            if button(ui, &p, Icon::ChevronLeft, label).clicked() {
                clicked += 1;
            }
        });
        harness.run();
        let node = harness.get_by_label(label);
        node.focus();
        harness.run();
        harness.key_press(egui::Key::Enter);
        harness.run();
        drop(harness);
        assert_eq!(clicked, 1);
    }
}
