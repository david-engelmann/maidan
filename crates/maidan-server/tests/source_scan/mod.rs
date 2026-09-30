//! Reading the workspace's own source for the contract tests that grep it.
//!
//! Splitting a file at its first `#[cfg(test)]` drops every line after it,
//! including real code after a test item inside an `impl`
//! (`land_gate_advisor.rs`) or after a `#[cfg(test)] use` at the top of a file
//! (`import.rs`, `openapi/mod.rs`). `without_tests` blanks each test item on
//! its own instead.

use std::path::{Path, PathBuf};

/// Every `.rs` file under `path`, or `path` itself when it is a file.
pub fn rust_files(path: &Path, out: &mut Vec<PathBuf>) {
    if path.is_file() {
        out.push(path.to_path_buf());
        return;
    }
    for entry in std::fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The index just past the item that starts at `from`: its `;`, or the brace
/// that closes its body. Skips comments and string, raw string and char
/// literals, so a `{` or `"` inside one does not count.
fn end_of_item(src: &[u8], from: usize) -> usize {
    let mut depth = 0usize;
    let mut i = from;
    while i < src.len() {
        match src[i] {
            b'/' if src.get(i + 1) == Some(&b'/') => {
                while i < src.len() && src[i] != b'\n' {
                    i += 1;
                }
            }
            b'"' => {
                i += 1;
                while i < src.len() && src[i] != b'"' {
                    if src[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            b'r' if src.get(i + 1).is_some_and(|b| *b == b'"' || *b == b'#') => {
                let hashes = src[i + 1..].iter().take_while(|b| **b == b'#').count();
                if src.get(i + 1 + hashes) == Some(&b'"') {
                    let close: Vec<u8> = std::iter::once(b'"')
                        .chain(std::iter::repeat_n(b'#', hashes))
                        .collect();
                    i += 2 + hashes;
                    while i < src.len() && !src[i..].starts_with(&close) {
                        i += 1;
                    }
                    i += close.len() - 1;
                }
            }
            b'\'' if src.get(i + 1) == Some(&b'\\') && src.get(i + 3) == Some(&b'\'') => i += 3,
            b'\'' if src.get(i + 2) == Some(&b'\'') => i += 2,
            b';' if depth == 0 => return i + 1,
            b'{' => depth += 1,
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    src.len()
}

/// `source` with every `#[cfg(test)]` item blanked out, line breaks kept so
/// line numbers still match.
pub fn without_tests(source: &str) -> String {
    let mut out = source.as_bytes().to_vec();
    let mut from = 0;
    while let Some(found) = source[from..].find("#[cfg(test)]") {
        let start = from + found;
        let end = end_of_item(source.as_bytes(), start + "#[cfg(test)]".len());
        for b in &mut out[start..end] {
            if *b != b'\n' {
                *b = b' ';
            }
        }
        from = end;
    }
    String::from_utf8(out).unwrap()
}

#[test]
fn blanking_test_items_keeps_the_code_after_them() {
    let src = "fn a() {}\n#[cfg(test)]\nmod t { fn x() { let _ = \"}\"; let _ = '{'; let _ = '\\''; } // }\n}\nfn b() { store.revoke(x); }\n#[cfg(test)]\nuse y;\nfn c() {}\n";
    let kept = without_tests(src);
    assert!(kept.contains("fn a()"));
    assert!(!kept.contains("mod t"));
    assert!(kept.contains("fn b() { store.revoke(x); }"));
    assert!(!kept.contains("use y"));
    assert!(kept.contains("fn c() {}"));
    assert_eq!(kept.lines().count(), src.lines().count());
}
