//! Sidebar and content frame.

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Frame, Layout, Margin, RichText, Role,
    Sense, Vec2, WidgetInfo,
};

use crate::app::navigation::Screen;
use crate::app::ui_state::UiState;
use crate::app::view_model::ViewModel;
use crate::i18n::strings;
use crate::ui::screens::{self, ScreenEvent};
use crate::ui::theme::{Palette, palette, typography};

const SIDEBAR_WIDTH: f32 = 220.0;
const CONTENT_MAX_WIDTH: f32 = 1100.0;

fn tint(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

fn nav_item(ui: &mut egui::Ui, p: &Palette, screen: Screen, label: &str, active: bool) -> bool {
    let size = Vec2::new(ui.available_width(), 36.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| WidgetInfo::selected(Role::Button, true, active, label));
    let fill = if active {
        tint(p.accent, 36)
    } else if response.hovered() {
        p.card_hover
    } else {
        Color32::TRANSPARENT
    };
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(10), fill);
    if active {
        let bar =
            egui::Rect::from_min_size(rect.left_top() + Vec2::new(0.0, 8.0), Vec2::new(3.0, 20.0));
        painter.rect_filled(bar, CornerRadius::same(2), p.accent);
    }
    let color = if active { p.accent } else { p.text };
    painter.text(
        rect.left_center() + Vec2::new(14.0, 0.0),
        Align2::LEFT_CENTER,
        format!("{}  {label}", screen.icon()),
        FontId::proportional(typography::BODY),
        color,
    );
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

/// What the shell and the active screen ask the app to do this frame.
#[derive(Debug, Default, PartialEq)]
pub struct ShellOutput {
    /// A sidebar item the user clicked.
    pub navigate: Option<Screen>,
    pub events: Vec<ScreenEvent>,
}

/// Draw the sidebar and the active screen.
pub fn show(ui: &mut egui::Ui, vm: &ViewModel, state: &mut UiState) -> ShellOutput {
    let p = palette(vm.theme());
    let s = strings(vm.language());
    let mut output = ShellOutput::default();

    egui::Panel::left("sidebar")
        .exact_size(SIDEBAR_WIDTH)
        .resizable(false)
        .frame(Frame::new().fill(p.panel).inner_margin(Margin::same(16)))
        .show(ui, |ui| {
            ui.label(
                RichText::new(s.app_name)
                    .text_style(typography::section())
                    .strong()
                    .color(p.accent),
            );
            ui.add_space(16.0);
            for screen in Screen::ALL {
                if nav_item(
                    ui,
                    &p,
                    screen,
                    screen.title(vm.language()),
                    state.screen == screen,
                ) {
                    output.navigate = Some(screen);
                }
            }
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
        });

    egui::CentralPanel::default()
        .frame(Frame::new().fill(p.bg).inner_margin(Margin::same(24)))
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_max_width(ui.available_width().min(CONTENT_MAX_WIDTH));
                    if vm.refresh_failed {
                        let mut retry = false;
                        Frame::new()
                            .fill(p.card)
                            .stroke(egui::Stroke::new(1.0, p.warning))
                            .corner_radius(CornerRadius::same(12))
                            .inner_margin(Margin::symmetric(14, 10))
                            .show(ui, |ui| {
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(RichText::new(s.view_refresh_failed).color(p.warning));
                                    if ui.button(s.retry).clicked() {
                                        retry = true;
                                    }
                                });
                            });
                        ui.add_space(12.0);
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
