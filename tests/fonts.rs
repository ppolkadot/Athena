//! Validates the committed font assets. The build never downloads a font.

use ttf_parser::{Face, name_id};

const REGULAR: &[u8] = include_bytes!("../assets/fonts/AthenaPolaris-Regular.otf");
const MEDIUM: &[u8] = include_bytes!("../assets/fonts/AthenaPolaris-Medium.otf");
const SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/AthenaPolaris-Semibold.otf");
const BOLD: &[u8] = include_bytes!("../assets/fonts/AthenaPolaris-Bold.otf");
const TABULAR: &[u8] = include_bytes!("../assets/fonts/AthenaPolaris-Tabular.otf");
const LICENSE: &str = include_str!("../assets/fonts/LICENSE.txt");

const FACES: [(&str, &[u8]); 5] = [
    ("Regular", REGULAR),
    ("Medium", MEDIUM),
    ("Semibold", SEMIBOLD),
    ("Bold", BOLD),
    ("Tabular", TABULAR),
];

fn name(face: &Face<'_>, id: u16) -> String {
    face.names()
        .into_iter()
        .find(|name| name.name_id == id && name.is_unicode())
        .and_then(|name| name.to_string())
        .unwrap_or_else(|| panic!("missing name id {id}"))
}

fn digit_widths(face: &Face<'_>) -> Vec<u16> {
    "0123456789"
        .chars()
        .map(|ch| {
            let glyph = face
                .glyph_index(ch)
                .unwrap_or_else(|| panic!("missing digit {ch}"));
            face.glyph_hor_advance(glyph)
                .unwrap_or_else(|| panic!("missing advance for {ch}"))
        })
        .collect()
}

#[test]
fn faces_are_subsets_with_the_required_metadata() {
    for (file, bytes) in FACES {
        assert!(bytes.len() < 250_000, "{file} is not a subset");
        let face = Face::parse(bytes, 0).unwrap_or_else(|err| panic!("{file}: {err:?}"));
        assert_eq!(
            name(&face, name_id::COPYRIGHT_NOTICE),
            "Salted Flutters Polaris - Created by ppoolar",
            "{file} lost the required metadata line"
        );
        // The upstream files have no typographic-family record (name ID 16); name ID 1 is the
        // family that applications actually show. The builder renames it.
        assert!(
            !name(&face, name_id::FAMILY).contains("Salted Flutters Polaris"),
            "{file} still uses the upstream primary family name"
        );
    }

    let regular = Face::parse(REGULAR, 0).unwrap();
    let tabular = Face::parse(TABULAR, 0).unwrap();
    assert_eq!(name(&regular, name_id::FAMILY), "Athena Polaris");
    assert_eq!(name(&tabular, name_id::FAMILY), "Athena Polaris Tabular");
}

#[test]
fn every_face_covers_athena_text() {
    const COVERAGE: &str = "R$ 1.234,56 − + ãõçáéíóúâêôàü • … × → ✓";
    for (file, bytes) in FACES {
        let face = Face::parse(bytes, 0).unwrap();
        for ch in COVERAGE.chars().filter(|ch| !ch.is_whitespace()) {
            assert!(
                face.glyph_index(ch).is_some(),
                "{file} lacks required glyph {ch:?}"
            );
        }
    }
}

#[test]
fn tabular_digits_share_one_advance() {
    let tabular = Face::parse(TABULAR, 0).unwrap();
    let widths = digit_widths(&tabular);
    assert!(
        widths.windows(2).all(|pair| pair[0] == pair[1]),
        "tabular digits differ: {widths:?}"
    );

    for (label, bytes) in [("Regular", REGULAR), ("Semibold", SEMIBOLD), ("Bold", BOLD)] {
        let face = Face::parse(bytes, 0).unwrap();
        let mut proportional = digit_widths(&face);
        proportional.sort_unstable();
        proportional.dedup();
        assert!(
            proportional.len() > 1,
            "{label} digits should stay proportional"
        );
    }
}

#[test]
fn upstream_license_is_shipped_unmodified() {
    assert_eq!(LICENSE.len(), 3_274);
    assert!(!LICENSE.contains('\r'));
    assert!(LICENSE.starts_with("SALTED FLUTTERS POLARIS FONT LICENSE"));
    assert!(LICENSE.contains("Salted Flutters Polaris - Created by ppoolar"));
}
