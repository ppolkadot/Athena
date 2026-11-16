//! Supplementary lexical guard (`contracts.md §6.4`): every read query outside the allow-list
//! must read from the `live_transactions` view rather than querying `transactions` directly.

use std::fs;
use std::path::{Path, PathBuf};

const ALLOW_LIST: [&str; 4] = [
    "src/infrastructure/db/trash_repo.rs",
    "src/infrastructure/db/transactions_repo.rs",
    "src/infrastructure/db/import_repo.rs",
    "src/infrastructure/db/integrity.rs",
];

fn strip_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;
    let mut block_depth = 0usize;

    while let Some(c) = chars.next() {
        if block_depth > 0 {
            if c == '/' && chars.peek() == Some(&'*') {
                chars.next();
                block_depth += 1;
            } else if c == '*' && chars.peek() == Some(&'/') {
                chars.next();
                block_depth -= 1;
                out.push(' ');
            } else if c == '\n' {
                out.push('\n');
            }
            continue;
        }
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        if c == '/' && chars.peek() == Some(&'/') {
            chars.next();
            for next in chars.by_ref() {
                if next == '\n' {
                    out.push('\n');
                    break;
                }
            }
            continue;
        }
        if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            block_depth = 1;
            continue;
        }
        if c == '"' {
            in_string = true;
        }
        out.push(c);
    }
    out
}

/// Strip `#[cfg(test)] mod tests { ... }` in `db/mod.rs` only, where low-level schema tests
/// delete directly from `transactions` to verify foreign-key cascades and v1 id preservation.
fn strip_db_mod_tests<'a>(rel: &str, source: &'a str) -> &'a str {
    if rel == "src/infrastructure/db/mod.rs"
        && let Some(idx) = source.find("#[cfg(test)]")
    {
        return &source[..idx];
    }
    source
}

fn collapse_whitespace(source: &str) -> String {
    let mut normalized = String::with_capacity(source.len());
    let mut prev_space = false;
    for ch in source.chars() {
        if ch.is_whitespace() {
            if !prev_space {
                normalized.push(' ');
                prev_space = true;
            }
        } else {
            normalized.push(ch.to_ascii_lowercase());
            prev_space = false;
        }
    }
    normalized
}

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn contains_direct_transactions_read(source: &str) -> bool {
    let stripped = strip_comments(source);
    let norm = collapse_whitespace(&stripped);
    let bytes = norm.as_bytes();
    for needle in ["from transactions", "join transactions"] {
        let mut offset = 0;
        while let Some(pos) = norm[offset..].find(needle) {
            let start = offset + pos;
            let end = start + needle.len();
            let left_ok = start == 0 || !is_ident(bytes[start - 1]);
            let right_ok = end == bytes.len() || !is_ident(bytes[end]);
            if left_ok && right_ok {
                return true;
            }
            offset = end;
        }
    }
    false
}

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
    for entry in entries {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn no_transactions_table() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    collect_rs_files(&root.join("src"), &mut files);
    files.sort();

    let mut violations = Vec::new();
    for path in files {
        let rel = path
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if ALLOW_LIST.contains(&rel.as_str()) {
            continue;
        }
        let raw = fs::read_to_string(&path).unwrap();
        let checked = strip_db_mod_tests(&rel, &raw);
        if contains_direct_transactions_read(checked) {
            violations.push(rel);
        }
    }
    assert!(
        violations.is_empty(),
        "only {ALLOW_LIST:?} may query `transactions` directly; found direct FROM/JOIN in: {violations:?}"
    );
}

#[test]
fn guard_catches_multiline_from_and_ignores_comments() {
    assert!(contains_direct_transactions_read(
        "let sql = \"SELECT * FROM\n  transactions WHERE id = 1\";"
    ));
    assert!(contains_direct_transactions_read(
        "let sql = \"SELECT * FROM allocations a JOIN\n transactions t ON t.id = a.tx_id\";"
    ));
    assert!(!contains_direct_transactions_read(
        "// SELECT * FROM transactions\n/* JOIN transactions */\nlet sql = \"SELECT * FROM live_transactions\";"
    ));
    assert!(!contains_direct_transactions_read(
        "let sql = \"SELECT * FROM live_transactions\";"
    ));
}
