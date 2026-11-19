//! Sidebar and content frame.

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, Frame, Layout, Margin, Rect, RichText, Role, Sense,
    Vec2, WidgetInfo,
};

use crate::app::navigation::Screen;
use crate::app::ui_state::UiState;
use crate::app::view_model::ViewModel;
use crate::i18n::strings;
use crate::ui::appearance::background::{self, Backdrop};
use crate::ui::icons;
use crate::ui::screens::{self, ScreenEvent};
use crate::ui::theme::effective::Material;
use crate::ui::theme::{Palette, palette, typography};
use crate::ui::tokens;

fn tint(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

/// One sidebar entry. With `rail` set only the icon is drawn; the localized name stays the
/// accessible label and hover text.
fn nav_item(
    ui: &mut egui::Ui,
    p: &Palette,
    screen: Screen,
    label: &str,
    active: bool,
    rail: bool,
) -> bool {
    let size = Vec2::new(ui.available_width(), tokens::control_height());
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| WidgetInfo::selected(Role::Button, true, active, label));
    let fill = if active {
        tint(p.accent, tokens::SELECTED_ALPHA)
    } else if response.hovered() {
        p.card_hover
    } else {
        Color32::TRANSPARENT
    };
    let radius = CornerRadius::same(tokens::CONTROL_RADIUS);
    let painter = ui.painter();
    painter.rect_filled(rect, radius, fill);
    if active {
        let bar =
            egui::Rect::from_min_size(rect.left_top() + Vec2::new(0.0, 8.0), Vec2::new(3.0, 20.0));
        painter.rect_filled(bar, CornerRadius::same(2), p.accent);
    }
    if response.has_focus() {
        painter.rect_stroke(
            rect,
            radius,
            egui::Stroke::new(2.0, p.focus),
            egui::StrokeKind::Inside,
        );
    }
    let color = if active { p.accent } else { p.text };
    let icon_rect = if rail {
        Rect::from_center_size(rect.center(), Vec2::splat(tokens::ICON_SIZE))
    } else {
        Rect::from_center_size(
            rect.left_center() + Vec2::new(14.0 + tokens::ICON_SIZE / 2.0, 0.0),
            Vec2::splat(tokens::ICON_SIZE),
        )
    };
    icons::paint_with(painter, icon_rect, screen.icon(), color);
    if !rail {
        painter.text(
            rect.left_center() + Vec2::new(14.0 + tokens::ICON_SIZE + 10.0, 0.0),
            Align2::LEFT_CENTER,
            label,
            typography::proportional(typography::BODY),
            color,
        );
    }
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let response = if rail {
        response.on_hover_text(label)
    } else {
        response
    };
    response.clicked()
}

/// What the shell and the active screen ask the app to do this frame.
#[derive(Debug, Default, PartialEq)]
pub struct ShellOutput {
    /// A sidebar item the user clicked.
    pub navigate: Option<Screen>,
    pub events: Vec<ScreenEvent>,
}

/// Draw the sidebar and the active screen on the plain canvas.
pub fn show(ui: &mut egui::Ui, vm: &ViewModel, state: &mut UiState) -> ShellOutput {
    show_with(ui, vm, state, &Backdrop::solid())
}

/// Draw the sidebar and the active screen over `backdrop`. A wallpaper spans the whole viewport
/// behind the chrome: the central canvas is transparent so every screen shows it, and the sidebar
/// is translucent (Tint) or opaque (Solid). Cards, rows, inputs and modals stay opaque.
pub fn show_with(
    ui: &mut egui::Ui,
    vm: &ViewModel,
    state: &mut UiState,
    backdrop: &Backdrop,
) -> ShellOutput {
    let p = palette(vm.theme());
    let wallpaper = backdrop.has_wallpaper();
    let (sidebar_fill, canvas_fill) = if wallpaper {
        background::paint(ui.painter(), ui.max_rect(), &p, backdrop);
        let alpha = match backdrop.material {
            Material::Tint => background::chrome_alpha(&p, backdrop.scrim),
            Material::Solid => u8::MAX,
        };
        (tint(p.panel, alpha), Color32::TRANSPARENT)
    } else {
        (p.panel, p.bg)
    };
    let s = strings(vm.language());
    let mut output = ShellOutput::default();
    // Collapse to a labeled icon rail when the full sidebar would leave under 480 pt of content.
    let rail = tokens::is_narrow(ui.available_width());
    let (sidebar_width, sidebar_margin, content_margin) = if rail {
        (tokens::RAIL_WIDTH, 8, 16)
    } else {
        (tokens::SIDEBAR_WIDTH, 16, 24)
    };

    egui::Panel::left("sidebar")
        .exact_size(sidebar_width)
        .resizable(false)
        .frame(
            Frame::new()
                .fill(sidebar_fill)
                .inner_margin(Margin::same(sidebar_margin)),
        )
        .show(ui, |ui| {
            if !rail {
                ui.label(
                    RichText::new(s.app_name)
                        .text_style(typography::section())
                        .strong()
                        .color(p.accent),
                );
            }
            ui.add_space(tokens::space_l());
            for screen in Screen::ALL {
                if nav_item(
                    ui,
                    &p,
                    screen,
                    screen.title(vm.language()),
                    state.screen == screen,
                    rail,
                ) {
                    output.navigate = Some(screen);
                }
            }
            if !rail {
                ui.with_layout(Layout::bottom_up(Align::LEFT), |ui| {
                    ui.label(
                        RichText::new(format!("{} {}", s.version, env!("CARGO_PKG_VERSION")))
                            .text_style(egui::TextStyle::Small)
                            .color(p.muted),
                    );
                    if let Some(name) = vm.owner_name() {
                        ui.label(RichText::new(name).color(p.text));
                    }
                });
            }
        });

    egui::CentralPanel::default()
        .frame(
            Frame::new()
                .fill(canvas_fill)
                .inner_margin(Margin::same(content_margin)),
        )
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_max_width(ui.available_width().min(tokens::CONTENT_MAX_WIDTH));
                    if vm.refresh_failed {
                        let mut retry = false;
                        Frame::new()
                            .fill(p.card)
                            .stroke(egui::Stroke::new(tokens::BORDER_WIDTH, p.warning))
                            .corner_radius(CornerRadius::same(tokens::CONTROL_RADIUS))
                            .inner_margin(Margin::symmetric(14, 10))
                            .show(ui, |ui| {
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(RichText::new(s.view_refresh_failed).color(p.warning));
                                    if ui.button(s.retry).clicked() {
                                        retry = true;
                                    }
                                });
                            });
                        ui.add_space(tokens::space_m());
                        output.events = screens::show(ui, vm, state);
                        if retry {
                            output.events.push(ScreenEvent::RetryRefresh);
                        }
                    } else {
                        output.events = screens::show(ui, vm, state);
                    }
                });
        });

    output
}
