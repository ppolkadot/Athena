//! Guards installer version alignment and CI packaging provenance contracts.

const CARGO_PKG_VERSION: &str = env!("CARGO_PKG_VERSION");
const INNO_SCRIPT: &str = include_str!("../installer/athena.iss");
const CI_WORKFLOW: &str = include_str!("../.github/workflows/ci.yml");

#[test]
fn installer_version_matches_cargo_package() {
    let expected = format!("#define AppVersion \"{CARGO_PKG_VERSION}\"");
    assert!(
        INNO_SCRIPT.lines().any(|line| line.trim() == expected),
        "installer/athena.iss must contain `{expected}`"
    );
    assert!(
        INNO_SCRIPT.contains("OutputBaseFilename=Athena-{#AppVersion}-setup"),
        "installer output filename must stay `Athena-<version>-setup.exe`"
    );
}

#[test]
fn ci_packaging_emits_build_info_and_checksums_preserving_asset_names() {
    for needle in [
        "ATHENA_INNO_SHA256",
        "Get-FileHash",
        "BUILD-INFO.txt",
        "GITHUB_REF",
        "GITHUB_SHA",
        "GITHUB_RUN_ID",
        "athena-$env:ATHENA_VERSION-windows-x64.exe",
        "Athena-$env:ATHENA_VERSION-setup.exe",
        "LICENSE.txt",
        "SHA256SUMS.txt",
        "target/package/BUILD-INFO.txt",
        "target/package/SHA256SUMS.txt",
    ] {
        assert!(
            CI_WORKFLOW.contains(needle),
            ".github/workflows/ci.yml is missing packaging provenance element `{needle}`"
        );
    }
}
