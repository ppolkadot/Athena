//! Confirmation modal showing an import summary before replacing the database (`§10.5`).

use eframe::egui::{self, Align, Button, Id, Layout, RichText, Vec2};

use crate::application::services::import::ImportPlan;
use crate::domain::settings::{Language, Theme};
use crate::i18n::strings;
use crate::ui::theme::{palette, typography};

#[derive(Debug, Clone, PartialEq)]
pub struct ImportPreviewDialog {
    pub plan: Box<ImportPlan>,
    pub current_people: usize,
    pub current_categories: usize,
    pub current_transactions: usize,
    pub current_templates: usize,
    pub current_goals: usize,
    pub current_closings: usize,
    pub current_trash: usize,
    pub snapshot_dir: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ImportPreviewChoice {
    Confirmed(Box<ImportPlan>),
    Cancelled,
}

/// Draw the import preview dialog. Returns the user's choice once they confirm or cancel (Esc
/// counts as cancel).
#[allow(clippy::must_use_candidate, clippy::too_many_lines)]
pub fn show(
    ctx: &egui::Context,
    dialog: &ImportPreviewDialog,
    theme: Theme,
    language: Language,
) -> Option<ImportPreviewChoice> {
    let p = palette(theme);
    let s = strings(language);
    let mut choice = None;
    let format_label = match dialog.plan.format_version {
        1 => s.import_preview_format_v1,
        2 => s.import_preview_format_v2,
        3 => s.import_preview_format_v3,
        4 => s.import_preview_format_v4,
        _ => s.import_preview_format_v5,
    };
    let meta = s
        .import_preview_meta
        .replace("{source}", &dialog.plan.source_name)
        .replace("{format}", format_label)
        .replace("{app_version}", &dialog.plan.app_version)
        .replace("{exported_at}", &dialog.plan.exported_at);
    let counts = s
        .import_preview_counts
        .replace("{in_people}", &dialog.plan.people.len().to_string())
        .replace("{in_categories}", &dialog.plan.categories.len().to_string())
        .replace(
            "{in_transactions}",
            &dialog.plan.transactions.len().to_string(),
        )
        .replace("{in_templates}", &dialog.plan.templates.len().to_string())
        .replace("{cur_templates}", &dialog.current_templates.to_string())
        .replace("{cur_people}", &dialog.current_people.to_string())
        .replace("{cur_categories}", &dialog.current_categories.to_string())
        .replace(
            "{cur_transactions}",
            &dialog.current_transactions.to_string(),
        )
        .replace("{cur_trash}", &dialog.current_trash.to_string());
    let history = s
        .import_preview_history
        .replace("{in_goals}", &dialog.plan.goals.len().to_string())
        .replace("{cur_goals}", &dialog.current_goals.to_string())
        .replace(
            "{in_contributions}",
            &dialog
                .plan
                .goals
                .iter()
                .map(|g| g.contributions.len())
                .sum::<usize>()
                .to_string(),
        )
        .replace("{in_closings}", &dialog.plan.closings.len().to_string())
        .replace("{cur_closings}", &dialog.current_closings.to_string());
    let snapshot = s
        .import_preview_snapshot
        .replace("{snapshot_dir}", &dialog.snapshot_dir);

    let modal = egui::Modal::new(Id::unique("import-preview-dialog")).show(ctx, |ui| {
        ui.set_width(440.0);
        ui.label(
            RichText::new(s.import_preview_title)
                .text_style(typography::section())
                .strong(),
        );
        ui.add_space(8.0);
        ui.label(RichText::new(meta).color(p.muted));
        if dialog.plan.used_timestamp_fallback {
            ui.add_space(6.0);
            ui.label(
                RichText::new(s.import_preview_timestamp_fallback)
                    .text_style(egui::TextStyle::Small)
                    .color(p.accent),
            );
        }
        if !dialog.plan.widened_categories.is_empty() {
            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    s.import_preview_widened
                        .replace("{names}", &dialog.plan.widened_categories.join(", ")),
                )
                .color(p.accent),
            );
        }
        ui.add_space(8.0);
        ui.label(RichText::new(counts).color(p.text));
        ui.add_space(4.0);
        ui.label(RichText::new(history).color(p.text));
        ui.add_space(6.0);
        // Appearance is replaced too: by the file's settings, or back to the defaults.
        let appearance_note = if dialog.plan.appearance.is_some() {
            s.ap_import_has
        } else {
            s.ap_import_none
        };
        ui.label(
            RichText::new(appearance_note)
                .text_style(egui::TextStyle::Small)
                .color(p.muted),
        );
        ui.add_space(8.0);
        ui.label(
            RichText::new(s.import_preview_consequence)
                .color(p.negative)
                .strong(),
        );
        ui.add_space(8.0);
        ui.label(
            RichText::new(snapshot)
                .text_style(egui::TextStyle::Small)
                .color(p.muted),
        );
        ui.add_space(16.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let confirm = Button::new(RichText::new(s.import_preview_confirm).color(p.bg).strong())
                .fill(p.negative)
                .min_size(Vec2::new(120.0, 36.0));
            if ui.add(confirm).clicked() {
                choice = Some(ImportPreviewChoice::Confirmed(dialog.plan.clone()));
            }
            if ui.add_sized([96.0, 36.0], Button::new(s.cancel)).clicked() {
                choice = Some(ImportPreviewChoice::Cancelled);
            }
        });
    });
    if choice.is_none() && modal.should_close() {
        choice = Some(ImportPreviewChoice::Cancelled);
    }
    choice
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::money::Money;
    use crate::domain::person::{Person, PersonKind};
    use crate::domain::settings::Profile;

    fn sample_dialog(fallback: bool) -> ImportPreviewDialog {
        ImportPreviewDialog {
            plan: Box::new(ImportPlan {
                source_name: "backup-2026.json".into(),
                format_version: if fallback { 1 } else { 2 },
                app_version: "1.0.2".into(),
                exported_at: "2026-11-10T10:00:00Z".into(),
                used_timestamp_fallback: fallback,
                widened_categories: vec![],
                profile: Profile {
                    start_balance: Money::ZERO,
                    theme: Theme::Dark,
                    language: Language::En,
                    onboarding_complete: true,
                },
                people: vec![Person {
                    id: 1,
                    name: "A3".into(),
                    kind: PersonKind::Owner,
                    archived: false,
                    created_at: "2026-11-10T10:00:00Z".into(),
                }],
                categories: vec![],
                transactions: vec![],
                templates: vec![],
                appearance: None,
                goals: Vec::new(),
                closings: Vec::new(),
            }),
            current_goals: 2,
            current_closings: 1,
            current_people: 3,
            current_categories: 12,
            current_transactions: 18,
            current_templates: 2,
            current_trash: 4,
            snapshot_dir: "C:/Athena/backups".into(),
        }
    }

    fn harness(
        dialog: ImportPreviewDialog,
        language: Language,
    ) -> egui_kittest::Harness<'static, Option<ImportPreviewChoice>> {
        egui_kittest::Harness::new_ui_state(
            move |ui, choice: &mut Option<ImportPreviewChoice>| {
                ui.ctx()
                    .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
                crate::ui::theme::apply(ui.ctx(), Theme::Dark);
                let style = ui.ctx().global_style();
                ui.set_style(style);
                if let Some(c) = show(ui.ctx(), &dialog, Theme::Dark, language) {
                    *choice = Some(c);
                }
            },
            None,
        )
    }

    #[test]
    fn preview_names_widened_categories_in_both_languages() {
        use egui_kittest::kittest::Queryable;
        for language in [Language::En, Language::PtBr] {
            let mut dialog = sample_dialog(false);
            dialog.plan.widened_categories = vec!["Food".into(), "Salary".into()];
            let expected = strings(language)
                .import_preview_widened
                .replace("{names}", "Food, Salary");
            let mut h = harness(dialog, language);
            h.run_steps(3);
            assert!(h.query_by_label(&expected).is_some());
        }
    }

    #[test]
    fn preview_shows_counts_trash_consequence_and_snapshot() {
        use egui_kittest::kittest::Queryable;

        // English with Format 1 timestamp fallback note.
        let dialog_en = sample_dialog(true);
        let mut harness_en = harness(dialog_en.clone(), Language::En);
        harness_en.run_steps(3);
        assert!(harness_en.query_by_label("Import profile?").is_some());
        assert!(
            harness_en
                .query_by_label(
                    "Source: backup-2026.json\nFormat: Format 1 (1.0) · Athena 1.0.2\nExported at: 2026-11-10T10:00:00Z"
                )
                .is_some()
        );
        assert!(
            harness_en
                .query_by_label("Entry times weren't recorded in this file.")
                .is_some()
        );
        assert!(
            harness_en
                .query_by_label(
                    "Incoming: 1 people · 0 categories · 0 entries · 0 templates\nCurrent: 3 people · 12 categories · 18 entries · 2 templates · 4 in Trash (will be emptied)"
                )
                .is_some()
        );
        assert!(
            harness_en
                .query_by_label("Replaces everything in Athena, including Trash. JSON files do not include Trash; use a database backup to restore Trash.")
                .is_some()
        );
        assert!(
            harness_en
                .query_by_label("A safety snapshot will be saved in:\nC:/Athena/backups")
                .is_some()
        );
        harness_en.get_by_label("Import").click();
        harness_en.run_steps(3);
        assert_eq!(
            *harness_en.state(),
            Some(ImportPreviewChoice::Confirmed(dialog_en.plan))
        );

        // Portuguese without fallback note + Escape cancels.
        let dialog_pt = sample_dialog(false);
        let mut harness_pt = harness(dialog_pt, Language::PtBr);
        harness_pt.run_steps(3);
        assert!(harness_pt.query_by_label("Importar perfil?").is_some());
        assert!(
            harness_pt
                .query_by_label(
                    "Origem: backup-2026.json\nFormato: Formato 2 · Athena 1.0.2\nExportado em: 2026-11-10T10:00:00Z"
                )
                .is_some()
        );
        assert!(
            harness_pt
                .query_by_label("Os horários dos lançamentos não foram gravados neste arquivo.")
                .is_none()
        );
        assert!(
            harness_pt
                .query_by_label(
                    "Novo: 1 pessoas · 0 categorias · 0 lançamentos · 0 modelos\nAtual: 3 pessoas · 12 categorias · 18 lançamentos · 2 modelos · 4 na Lixeira (será esvaziada)"
                )
                .is_some()
        );
        assert!(
            harness_pt
                .query_by_label("Substitui tudo no Athena, incluindo a Lixeira. Arquivos JSON não incluem a Lixeira; use um backup do banco para restaurar a Lixeira.")
                .is_some()
        );
        assert!(
            harness_pt
                .query_by_label("Um snapshot de segurança será salvo em:\nC:/Athena/backups")
                .is_some()
        );
        harness_pt.key_press(egui::Key::Escape);
        harness_pt.run_steps(3);
        assert_eq!(*harness_pt.state(), Some(ImportPreviewChoice::Cancelled));
    }
}
