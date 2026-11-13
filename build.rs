//! Windows resources for the release executable: icon, product name and version info.
//! A failure here (for example no resource compiler on the machine) only warns; the app itself
//! does not depend on it.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=build.rs");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut resource = winresource::WindowsResource::new();
    resource
        .set_icon("assets/icon.ico")
        .set("ProductName", "Athena")
        .set("FileDescription", "Athena - household finance")
        .set("InternalName", "athena")
        .set("OriginalFilename", "athena.exe")
        .set("LegalCopyright", "Local-first. No cloud. No accounts.");
    if let Err(err) = resource.compile() {
        println!("cargo:warning=could not embed the Windows icon and version info: {err}");
    }
}
