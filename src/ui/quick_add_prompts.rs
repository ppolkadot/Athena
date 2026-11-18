//! Child prompts of the Quick Add modal: discard changes, possible duplicate, and name a template.
//! Each is its own modal layered above the form, so Escape and outside clicks close only the
//! prompt, and keyboard focus starts on the safe choice.

use eframe::egui::{self, Align, Button, Id, Key, Layout, Modifiers, RichText};

use crate::app::view_model::ViewModel;
use crate::i18n::{format_date, strings, template_error_message};
use crate::ui::quick_add::{DuplicateConsent, PromptError, TemplatePrompt};
use crate::ui::theme::palette;

/// What the user chose in a two-button prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptChoice {
    /// The safe choice: close the prompt and return to the form.
    KeepEditing,
    /// The confirming choice.
    Confirm,
}

fn buttons(
    ui: &mut egui::Ui,
    vm: &ViewModel,
    safe_label: &str,
    confirm_label: &str,
    danger: bool,
    focus_safe: bool,
) -> Option<PromptChoice> {
    let p = palette(vm.theme());
    let mut choice = None;
    ui.add_space(16.0);
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        let fill = if danger { p.negative } else { p.accent };
        let confirm =
            Button::new(RichText::new(confirm_label).color(p.on_accent).strong()).fill(fill);
        if ui.add_sized([130.0, 36.0], confirm).clicked() {
            choice = Some(PromptChoice::Confirm);
        }
        let safe = ui.add_sized([130.0, 36.0], Button::new(safe_label));
        if focus_safe {
            safe.request_focus();
        }
        if safe.clicked() {
            choice = Some(PromptChoice::KeepEditing);
        }
    });
    choice
}

/// "Discard this entry?" with Keep editing (focused) and Discard.
#[must_use]
pub fn discard(ctx: &egui::Context, vm: &ViewModel, focus_safe: bool) -> Option<PromptChoice> {
    let s = strings(vm.language());
    let mut choice = None;
    let modal = egui::Modal::new(Id::unique("quick-add-discard")).show(ctx, |ui| {
        ui.set_width(380.0_f32.min(ctx.content_rect().width() - 48.0));
        ui.label(
            RichText::new(s.qa_discard_title)
                .text_style(egui::TextStyle::Heading)
                .strong(),
        );
        ui.add_space(8.0);
        ui.label(s.qa_discard_body);
        choice = buttons(ui, vm, s.qa_keep_editing, s.qa_discard, true, focus_safe);
    });
    if choice.is_none() && modal.should_close() {
        choice = Some(PromptChoice::KeepEditing);
    }
    choice
}

/// "Possible duplicate" naming the first match, its date and amount, and the match count.
#[must_use]
pub fn duplicate(
    ctx: &egui::Context,
    vm: &ViewModel,
    consent: &DuplicateConsent,
    focus_safe: bool,
) -> Option<PromptChoice> {
    let s = strings(vm.language());
    let template = if consent.count > 1 {
        s.qa_dup_body_many
    } else {
        s.qa_dup_body_one
    };
    let body = template
        .replace("{count}", &consent.count.to_string())
        .replace("{date}", &format_date(consent.first.date, vm.language()))
        .replace("{amount}", &consent.first.amount.format_brl())
        .replace("{title}", &consent.first.title);
    let mut choice = None;
    let modal = egui::Modal::new(Id::unique("quick-add-duplicate")).show(ctx, |ui| {
        ui.set_width(400.0_f32.min(ctx.content_rect().width() - 48.0));
        ui.label(
            RichText::new(s.qa_dup_title)
                .text_style(egui::TextStyle::Heading)
                .strong(),
        );
        ui.add_space(8.0);
        ui.add(egui::Label::new(body).wrap());
        choice = buttons(
            ui,
            vm,
            s.qa_keep_editing,
            s.qa_save_anyway,
            false,
            focus_safe,
        );
    });
    if choice.is_none() && modal.should_close() {
        choice = Some(PromptChoice::KeepEditing);
    }
    choice
}

/// What the user chose in the template-name prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameChoice {
    Cancel,
    Save,
}

/// Ask for a template name. Enter in the field saves once; Escape cancels.
pub fn template_name(
    ctx: &egui::Context,
    vm: &ViewModel,
    prompt: &mut TemplatePrompt,
    focus_field: bool,
) -> Option<NameChoice> {
    let s = strings(vm.language());
    let p = palette(vm.theme());
    let mut choice = None;
    let modal = egui::Modal::new(Id::unique("quick-add-template-name")).show(ctx, |ui| {
        ui.set_width(380.0_f32.min(ctx.content_rect().width() - 48.0));
        ui.label(
            RichText::new(s.qa_save_as_template)
                .text_style(egui::TextStyle::Heading)
                .strong(),
        );
        ui.add_space(8.0);
        let caption =
            ui.label(RichText::new(s.qa_template_name).text_style(egui::TextStyle::Small));
        let field = ui.add(
            egui::TextEdit::singleline(&mut prompt.name)
                .hint_text(s.qa_template_name_hint)
                .desired_width(f32::INFINITY)
                .margin(egui::Vec2::new(12.0, 10.0)),
        );
        let field = field.labelled_by(caption.id);
        if focus_field {
            field.request_focus();
        }
        if field.changed() {
            prompt.error = None;
        }
        if let Some(error) = &prompt.error {
            let text = match error {
                PromptError::Template(e) => template_error_message(e, vm.language()),
                PromptError::Failed => s.qa_template_failed.to_string(),
            };
            ui.label(RichText::new(text).color(p.negative));
        }
        let enter = field.lost_focus()
            && ui.input(|i| {
                i.events.iter().any(|e| {
                    matches!(
                        e,
                        egui::Event::Key {
                            key: Key::Enter,
                            pressed: true,
                            repeat: false,
                            ..
                        }
                    )
                })
            });
        let _ = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
        ui.add_space(16.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let save = Button::new(
                RichText::new(s.qa_template_save)
                    .color(p.on_accent)
                    .strong(),
            )
            .fill(p.accent);
            if ui.add_sized([130.0, 36.0], save).clicked() || enter {
                choice = Some(NameChoice::Save);
            }
            if ui.add_sized([96.0, 36.0], Button::new(s.cancel)).clicked() {
                choice = Some(NameChoice::Cancel);
            }
        });
    });
    if choice.is_none() && modal.should_close() {
        choice = Some(NameChoice::Cancel);
    }
    choice
}
