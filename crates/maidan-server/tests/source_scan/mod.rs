//! Reading the workspace's own source for the contract tests that grep it.
//!
//! Splitting a file at its first `#[cfg(test)]` drops every line after it,
//! including real code after a test item inside an `impl`
//! (`land_gate_advisor.rs`) or after a `#[cfg(test)] use` at the top of a file
//! (`import.rs`, `openapi/mod.rs`). `without_tests` blanks each test item on
//! its own instead, and only a marker that lexically sits in code: one inside
//! a comment or a string, including a raw string, is not a test item.

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

/// Index just past a comment or literal that starts at `i`, or `i` when the
/// byte is code. Skips line comments, nested block comments, cooked and raw
/// strings (including `b`/`c` prefixes) and char literals, so a `{` or the
/// test marker inside one does not count.
fn consume_non_code(src: &[u8], i: usize) -> usize {
    if i >= src.len() {
        return i;
    }
    if src[i] == b'/' && src.get(i + 1) == Some(&b'/') {
        let mut j = i + 2;
        while j < src.len() && src[j] != b'\n' {
            j += 1;
        }
        return j;
    }
    if src[i] == b'/' && src.get(i + 1) == Some(&b'*') {
        return skip_block_comment(src, i);
    }
    try_skip_literal(src, i).unwrap_or(i)
}

fn skip_block_comment(src: &[u8], i: usize) -> usize {
    let mut depth = 1usize;
    let mut k = i + 2;
    while k < src.len() && depth > 0 {
        if src[k] == b'/' && src.get(k + 1) == Some(&b'*') {
            depth += 1;
            k += 2;
        } else if src[k] == b'*' && src.get(k + 1) == Some(&b'/') {
            depth -= 1;
            k += 2;
        } else {
            k += 1;
        }
    }
    k
}

/// A string or char literal starting at `i`, if that is what the bytes are.
/// A lifetime (`'a`) and a raw identifier (`r#foo`) are code and return none.
fn try_skip_literal(src: &[u8], i: usize) -> Option<usize> {
    let b0 = *src.get(i)?;
    if b0 == b'\'' {
        return skip_char(src, i);
    }
    let mut j = i;
    if b0 == b'b' || b0 == b'c' {
        j += 1;
        if src.get(j) == Some(&b'\'') && b0 == b'b' {
            return skip_char(src, j);
        }
    }
    if src.get(j) == Some(&b'r') {
        let mut k = j + 1;
        let mut hashes = 0usize;
        while src.get(k) == Some(&b'#') {
            hashes += 1;
            k += 1;
        }
        if src.get(k) == Some(&b'"') {
            return Some(skip_raw_string(src, k + 1, hashes));
        }
        return None;
    }
    if src.get(j) == Some(&b'"') && (j == i || matches!(b0, b'b' | b'c')) {
        return Some(skip_cooked_string(src, j));
    }
    None
}

fn skip_cooked_string(src: &[u8], quote_at: usize) -> usize {
    let mut i = quote_at + 1;
    while i < src.len() {
        match src[i] {
            b'\\' => i = i.saturating_add(2),
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    src.len()
}

fn skip_raw_string(src: &[u8], content_start: usize, hashes: usize) -> usize {
    let mut closer = vec![b'"'];
    closer.extend(std::iter::repeat_n(b'#', hashes));
    let mut i = content_start;
    while i < src.len() && !src[i..].starts_with(&closer) {
        i += 1;
    }
    if i >= src.len() {
        src.len()
    } else {
        i + closer.len()
    }
}

fn skip_char(src: &[u8], i: usize) -> Option<usize> {
    match src.get(i + 1)? {
        b'\\' if src.get(i + 2) == Some(&b'u') && src.get(i + 3) == Some(&b'{') => {
            let end = src[i + 4..].iter().position(|b| *b == b'}')?;
            let close = i + 4 + end;
            if src.get(close + 1) == Some(&b'\'') {
                Some(close + 2)
            } else {
                None
            }
        }
        b'\\' if src.get(i + 2) == Some(&b'x') && src.get(i + 5) == Some(&b'\'') => Some(i + 6),
        b'\\' if src.get(i + 3) == Some(&b'\'') => Some(i + 4),
        _ if src.get(i + 2) == Some(&b'\'') => Some(i + 3),
        _ => None,
    }
}

fn find_in_code(src: &[u8], mut i: usize, needle: &[u8]) -> Option<usize> {
    while i < src.len() {
        let next = consume_non_code(src, i);
        if next != i {
            i = next;
            continue;
        }
        if src[i..].starts_with(needle) {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// The index just past the item that starts at `from`: its `;`, or the brace
/// that closes its body. Skips comments and string, raw string and char
/// literals, so a `{` or `"` inside one does not count.
fn end_of_item(src: &[u8], mut i: usize) -> usize {
    let mut depth = 0usize;
    while i < src.len() {
        let next = consume_non_code(src, i);
        if next != i {
            i = next;
            continue;
        }
        match src[i] {
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
/// line numbers still match. A marker inside a comment or a literal is left
/// alone, and the code after it is not blanked.
pub fn without_tests(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = bytes.to_vec();
    let needle = b"#[cfg(test)]";
    let mut from = 0;
    while let Some(start) = find_in_code(bytes, from, needle) {
        let end = end_of_item(bytes, start + needle.len());
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

#[test]
fn a_marker_in_a_comment_or_string_does_not_blank_the_code_after_it() {
    let src = "\
fn keep() { store.revoke(x); }\n\
fn live() {} // #[cfg(test)]\n\
fn after_line_comment() {}\n\
/* #[cfg(test)] { } */\n\
fn after_block_comment() {}\n\
fn raw() { let _ = r#\"#[cfg(test)]\"#; }\n\
fn after_raw() { store.revoke(y); }\n\
fn cooked() { let _ = \"#[cfg(test)]\"; }\n\
#[cfg(test)]\n\
fn hidden() {\n\
    /* } */\n\
    let _ = 1;\n\
}\n\
fn after_test() { store.revoke(z); }\n\
";
    let kept = without_tests(src);
    assert!(kept.contains("fn keep() { store.revoke(x); }"));
    assert!(kept.contains("fn after_line_comment()"));
    assert!(kept.contains("fn after_block_comment()"));
    assert!(kept.contains("fn raw()"));
    assert!(kept.contains("r#\"#[cfg(test)]\"#"));
    assert!(kept.contains("fn after_raw() { store.revoke(y); }"));
    assert!(kept.contains("fn cooked()"));
    assert!(!kept.contains("fn hidden"));
    assert!(!kept.contains("let _ = 1"));
    assert!(kept.contains("fn after_test() { store.revoke(z); }"));
    assert_eq!(kept.lines().count(), src.lines().count());
}
