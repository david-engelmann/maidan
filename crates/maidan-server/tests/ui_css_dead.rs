//! Every class and id a `board.css` selector names is used by the page.
//!
//! Dead rules piled up once already and were deleted by hand. This check
//! keeps them from coming back: a selector naming a class or id that neither
//! `index.html` nor any `/ui` module mentions fails, so a rename or removal in
//! the markup takes its CSS with it.

use std::collections::BTreeSet;
use std::path::PathBuf;

fn static_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("static")
}

/// `index.html` and every `/ui` module, read from disk so a new module is
/// covered without editing this test.
fn sources() -> String {
    let dir = static_dir();
    let mut out = std::fs::read_to_string(dir.join("index.html")).expect("index.html");
    let mut modules: Vec<PathBuf> = std::fs::read_dir(dir.join("ui"))
        .expect("static/ui")
        .map(|entry| entry.expect("dir entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "js"))
        .collect();
    modules.sort();
    for module in modules {
        out.push('\n');
        out.push_str(&std::fs::read_to_string(&module).expect("ui module"));
    }
    out
}

fn strip_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        rest = match rest[start + 2..].find("*/") {
            Some(end) => &rest[start + 2 + end + 2..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

/// The selector preludes of every style rule: the text before a `{` that is
/// not an at-rule. Declarations end in `;` or `}`, which reset the buffer.
fn selectors(css: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    for c in strip_comments(css).chars() {
        match c {
            '{' => {
                let prelude = buf.trim();
                if !prelude.is_empty() && !prelude.starts_with('@') {
                    out.push(prelude.to_string());
                }
                buf.clear();
            }
            ';' | '}' => buf.clear(),
            _ => buf.push(c),
        }
    }
    out
}

/// Drop attribute selectors and quoted strings, whose `#` and `.` are values.
fn without_attributes(selector: &str) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    for c in selector.chars() {
        match c {
            '[' => depth += 1,
            ']' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

/// Each `.class` and `#id` a selector names, with its sigil.
fn names(selector: &str) -> Vec<(char, String)> {
    let text = without_attributes(selector);
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let sigil = chars[i];
        let starts_name = chars
            .get(i + 1)
            .is_some_and(|c| c.is_ascii_alphabetic() || *c == '_' || *c == '-');
        if (sigil == '.' || sigil == '#') && starts_name {
            let mut j = i + 1;
            while j < chars.len() && is_name_char(chars[j]) {
                j += 1;
            }
            out.push((sigil, chars[i + 1..j].iter().collect()));
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

/// The name appears as a whole token, or the code builds it from a literal
/// prefix (`` `card-status-${s}` `` or `"card-status-" + s`).
fn used(name: &str, sources: &str) -> bool {
    let whole = sources.match_indices(name).any(|(at, _)| {
        let before = sources[..at].chars().next_back();
        let after = sources[at + name.len()..].chars().next();
        !before.is_some_and(is_name_char) && !after.is_some_and(is_name_char)
    });
    whole
        || name.match_indices('-').any(|(at, _)| {
            let prefix = &name[..=at];
            [
                format!("{prefix}${{"),
                format!("{prefix}\" +"),
                format!("{prefix}' +"),
            ]
            .iter()
            .any(|built| sources.contains(built.as_str()))
        })
}

#[test]
fn every_class_and_id_in_board_css_is_used() {
    let css = std::fs::read_to_string(static_dir().join("ui/board.css")).expect("board.css");
    let sources = sources();
    let named: Vec<(char, String)> = selectors(&css)
        .iter()
        .flat_map(|selector| names(selector))
        .collect();
    // A scanner that stopped finding names would pass vacuously.
    assert!(
        named.len() > 100,
        "only {} names found in board.css",
        named.len()
    );
    let dead: BTreeSet<String> = named
        .into_iter()
        .filter(|(_, name)| !used(name, &sources))
        .map(|(sigil, name)| format!("{sigil}{name}"))
        .collect();
    assert!(
        dead.is_empty(),
        "board.css styles classes or ids that nothing in index.html or static/ui/*.js uses; delete the rules or fix the names: {dead:?}"
    );
}

#[test]
fn the_scan_finds_names_and_skips_values() {
    let css = "/* .gone */ @media (max-width: 720px) { #board .card:not(.mine) > a[href=\"#x.y\"] { color: #fff; margin: .5rem; } }";
    let found: Vec<String> = selectors(css)
        .iter()
        .flat_map(|selector| names(selector))
        .map(|(sigil, name)| format!("{sigil}{name}"))
        .collect();
    assert_eq!(found, ["#board", ".card", ".mine"]);
    assert!(used(
        "card-status-done",
        "el.className = `card-status-${status}`;"
    ));
    assert!(used("mine", "chip.className = \"reaction-chip mine\";"));
    assert!(!used("mine", "const mineCount = 1;"));
}
