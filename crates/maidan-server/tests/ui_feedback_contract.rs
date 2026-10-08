//! The `/ui` has one feedback surface. `showError(message, severity)` puts
//! an error, a warning or a success in `#toasts`; the old `setStatus` and its
//! `#status` paragraph are gone, so a message cannot land on a line at the
//! bottom of a closed panel where nobody reads it. The toast region and the
//! session line are live regions, so a screen reader hears both.

use std::path::Path;

const HTML: &str = include_str!("../static/index.html");

/// Every page module, read from disk, so a module added later is covered too.
fn modules() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("static/ui");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("static/ui") {
        let path = entry.expect("entry").path();
        let is_page_code = path
            .extension()
            .is_some_and(|ext| ext == "js" || ext == "mjs");
        if is_page_code {
            let text = std::fs::read_to_string(&path).expect("read module");
            out.push((path.display().to_string(), text));
        }
    }
    assert!(
        out.len() > 10,
        "found the page modules in {}",
        dir.display()
    );
    out
}

/// The opening tag of the element with this id.
fn tag_with_id<'a>(html: &'a str, id: &str) -> &'a str {
    let at = html
        .find(&format!("id=\"{id}\""))
        .unwrap_or_else(|| panic!("#{id} is on the page"));
    let start = html[..at].rfind('<').expect("tag start");
    let end = at + html[at..].find('>').expect("tag end");
    &html[start..=end]
}

#[test]
fn set_status_and_the_status_line_are_gone() {
    for (path, text) in modules() {
        assert!(
            !text.contains("setStatus"),
            "{path} still names setStatus; use showError(message, severity)"
        );
        assert!(
            !text.contains("getElementById(\"status\")"),
            "{path} still writes the removed #status line"
        );
    }
    assert!(
        !HTML.contains("id=\"status\""),
        "the #status paragraph is gone: showError is the one feedback surface"
    );
}

#[test]
fn show_error_takes_a_severity() {
    let feedback = include_str!("../static/ui/feedback.js");
    assert!(
        feedback.contains("function showError(message, severity = \"error\")"),
        "showError takes a severity and defaults to an error"
    );
    assert!(
        feedback.contains("const SEVERITIES = [\"error\", \"warning\", \"success\"];"),
        "the severities are error, warning and success"
    );
    assert!(
        feedback.contains(
            "toast.setAttribute(\"role\", level === \"success\" ? \"status\" : \"alert\");"
        ),
        "an error or a warning interrupts; a success is a polite status"
    );
}

#[test]
fn the_feedback_line_and_the_session_line_are_live_regions() {
    for id in ["toasts", "session-status"] {
        let tag = tag_with_id(HTML, id);
        assert!(
            tag.contains("role=\"status\"") && tag.contains("aria-live=\"polite\""),
            "#{id} is a polite live region: {tag}"
        );
    }
    assert!(
        tag_with_id(HTML, "toasts").contains("aria-atomic=\"false\""),
        "a new toast is read on its own, not with every toast still showing"
    );
}
