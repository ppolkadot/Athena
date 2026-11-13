// Release builds are a GUI program with no console window; startup failures are shown in a
// message box instead. Debug builds keep the console so logs are visible.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Err(err) = athena::run() {
        eprintln!("startup failed: {err}");
        let language = athena::domain::settings::Language::from_system();
        rfd::MessageDialog::new()
            .set_title("Athena")
            .set_description(athena::i18n::startup_error_message(&err, language))
            .set_level(rfd::MessageLevel::Error)
            .show();
        std::process::exit(1);
    }
}
