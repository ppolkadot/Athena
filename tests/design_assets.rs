//! Guards the designer handoff: the brief is a product constraint, not loose copy.

const BRIEF: &str = include_str!("../docs/design/athena-designer-brief.md");

#[test]
fn designer_brief_fits_the_handoff_limit() {
    let brief = BRIEF.replace("\r\n", "\n");
    let count = brief.chars().count();
    assert!(
        count <= 4_000,
        "brief is {count} characters; limit is 4,000"
    );
    assert!(count >= 3_500, "brief looks truncated: {count} characters");
    for needle in ["Salted Flutters Polaris", "R$", "pt-BR", "tnum"] {
        assert!(brief.contains(needle), "brief lost {needle}");
    }
}
