//! The look shared by the server-rendered OAuth pages: consent and connected
//! apps. They are the first Maidan screens a person meets when an outside app
//! connects, so they carry the console's tokens and the mark. Dark, as the
//! console is by default: with no script, a page cannot read a stored choice.
//!
//! The stylesheet is one inline `<style>` element. The page CSP keeps
//! `style-src 'self'` and admits that element by its SHA-256 alone, so no
//! other inline style runs and no stylesheet route is needed.

use std::sync::LazyLock;

use base64::{engine::general_purpose::STANDARD, Engine};
use sha2::{Digest, Sha256};

use crate::app::OAUTH_PAGE_CSP;

const CSS: &str = r#"
:root,:root[data-theme="dark"]{color-scheme:dark;--paper:#0c0d0f;--card:#16171b;--ink:#ecebe7;--muted:#9fa3ab;--line:rgba(236,240,248,.09);--line-strong:rgba(236,240,248,.16);--hover:rgba(236,240,248,.055);--signal:#ff6a2b;--danger:#ff6f64;--hi:rgba(255,255,255,.055);--mark:#ff6a2b;--primary:linear-gradient(180deg,#ff7b40,#f45a1d);--primary-line:#f45a1d;--primary-ink:#1b0b03;--card-shadow:0 1px 0 var(--hi) inset,0 30px 80px -30px rgba(0,0,0,.9);--page-bg:radial-gradient(900px 480px at 85% -10%,rgba(255,112,52,.08),transparent 62%),#0c0d0f}
:root[data-theme="light"]{color-scheme:light;--paper:#f2eee6;--card:#faf8f3;--ink:#151411;--muted:#5c5850;--line:rgba(21,20,17,.12);--line-strong:rgba(21,20,17,.22);--hover:rgba(21,20,17,.05);--signal:#e8501c;--danger:#b4231a;--hi:#fdfcf9;--mark:#151411;--primary:#151411;--primary-line:#151411;--primary-ink:#f2eee6;--card-shadow:0 1px 0 var(--hi) inset,0 30px 80px -40px rgba(21,20,17,.45);--page-bg:#f2eee6}
:root{--sans:Geist,ui-sans-serif,system-ui,-apple-system,sans-serif;--mono:ui-monospace,"SF Mono",Menlo,monospace;--ring:0 0 0 3px color-mix(in srgb,var(--signal) 35%,transparent)}
*{box-sizing:border-box}
html,body{margin:0;background:var(--paper);color:var(--ink);font:15px/1.5 var(--sans);-webkit-font-smoothing:antialiased}
body{background:var(--page-bg);min-height:100vh;display:flex;flex-direction:column;align-items:center;padding:max(16px, 6vh, env(safe-area-inset-top)) max(16px, env(safe-area-inset-right)) max(32px, env(safe-area-inset-bottom)) max(16px, env(safe-area-inset-left))}
.brand{display:flex;align-items:center;gap:8px;font-weight:600;letter-spacing:-.01em;min-height:44px;min-width:44px;padding:8px 12px;margin-bottom:12px;border-radius:999px;color:var(--ink);text-decoration:none}
.brand svg{width:22px;height:22px;color:var(--mark)}
main{width:100%;max-width:480px;background:var(--card);border:1px solid var(--line);border-radius:16px;box-shadow:var(--card-shadow);padding:28px}
main.wide{max-width:560px}
h1{font-size:22px;line-height:1.25;letter-spacing:-.02em;margin:0 0 8px;font-weight:600}
h2{font-size:12px;text-transform:uppercase;letter-spacing:.06em;color:var(--muted);font-weight:600;margin:24px 0 8px}
p{margin:0 0 12px}
.sub{color:var(--muted)}
.sub b{color:var(--ink);font-weight:600}
ul.caps{list-style:none;margin:0;padding:0;border:1px solid var(--line);border-radius:12px}
ul.caps li{display:flex;justify-content:space-between;align-items:baseline;gap:12px;padding:10px 14px;border-top:1px solid var(--line)}
ul.caps li:first-child{border-top:0}
ul.caps code{font:12px var(--mono);color:var(--muted);white-space:nowrap}
.note{font-size:13px;color:var(--muted);margin-top:20px}
.warn{font-size:14px;margin:16px 0 0;padding:12px 14px;border:1px solid color-mix(in srgb,var(--signal) 45%,transparent);border-radius:12px;background:color-mix(in srgb,var(--signal) 8%,transparent)}
.warn b{font-weight:600}
.actions{display:flex;gap:10px;margin-top:20px}
.actions form{display:contents}
button{flex:1;height:44px;padding:0 18px;border-radius:999px;font:600 15px var(--sans);cursor:pointer;border:1px solid var(--line-strong);background:transparent;color:var(--ink);transition:background .12s}
button:hover{background:var(--hover)}
button:focus-visible{outline:none;box-shadow:var(--ring)}
button.primary{background:var(--primary);color:var(--primary-ink);border-color:var(--primary-line)}
button.primary:hover{opacity:.9}
button.danger{flex:0 0 auto;color:var(--danger)}
.foot{font-size:13px;color:var(--muted);text-align:center;margin-top:16px}
.foot a{display:inline-flex;align-items:center;justify-content:center;min-height:44px;min-width:44px;padding:0 10px;border-radius:8px;color:inherit}
.note a{color:inherit}
a:focus-visible{outline:none;box-shadow:var(--ring);border-radius:6px}
.brand:focus-visible{border-radius:999px}
section.app{border-top:1px solid var(--line);padding:18px 0 4px}
section.app:first-of-type{border-top:0}
section.app header{display:flex;justify-content:space-between;align-items:center;gap:12px;margin-bottom:10px}
section.app h2{margin:0;font-size:16px;font-weight:600;text-transform:none;letter-spacing:normal;color:var(--ink)}
section.app .id{font:12px var(--mono);color:var(--muted);word-break:break-all}
.empty{color:var(--muted);text-align:center;padding:24px 0 12px}
@media (max-width:480px){main{padding:22px 18px}ul.caps li{flex-direction:column;gap:2px}section.app header{flex-wrap:wrap}}
@media (prefers-reduced-motion:reduce){button{transition:none}}
"#;

/// The Maidan mark, as the console draws it.
const MARK: &str = r#"<svg viewBox="0 0 64 64" aria-hidden="true" fill="currentColor"><path d="M38.95,32.85L48.07,37.53L59.65,35.70L59.74,34.95L48.69,28.76Z"/><path d="M36.31,37.52L39.45,47.28L46.40,51.08L47.18,50.46L46.09,41.51Z"/><path d="M31.15,38.95L26.47,48.07L28.30,59.65L29.05,59.74L35.24,48.69Z"/><path d="M26.48,36.31L16.72,39.45L12.92,46.40L13.54,47.18L22.49,46.09Z"/><path d="M25.05,31.15L15.93,26.47L4.35,28.30L4.26,29.05L15.31,35.24Z"/><path d="M27.69,26.48L24.55,16.72L17.60,12.92L16.82,13.54L17.91,22.49Z"/><path d="M32.85,25.05L37.53,15.93L35.70,4.35L34.95,4.26L28.76,15.31Z"/><path d="M37.52,27.69L47.28,24.55L51.08,17.60L50.46,16.82L41.51,17.91Z"/></svg>"#;

static STYLE_SOURCE: LazyLock<String> = LazyLock::new(|| {
    format!(
        "style-src 'self' 'sha256-{}'",
        STANDARD.encode(Sha256::digest(CSS.as_bytes()))
    )
});

/// The OAuth page CSP with the stylesheet's hash admitted.
pub(super) fn csp() -> String {
    OAUTH_PAGE_CSP.replacen("style-src 'self'", &STYLE_SOURCE, 1)
}

/// A whole page: the mark, then `body` on one card. `title` is escaped here;
/// `body` is markup the caller has already escaped.
pub(super) fn render(title: &str, class: &str, body: &str) -> String {
    format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover"><meta name="color-scheme" content="dark"><meta name="theme-color" content="#0c0d0f"><meta name="referrer" content="no-referrer"><title>{title} · Maidan</title><style>{CSS}</style></head>
<body>
<a class="brand" href="/ui/">{MARK}<span>Maidan</span></a>
<main class="{class}">
{body}
</main>
</body>
</html>"##,
        title = html_escape(title),
    )
}

/// What a capability lets an app do, in words a person approving it reads.
/// An unknown string shows as itself.
pub(super) fn describe(capability: &str) -> &str {
    use maidan_auth::capability as c;
    match capability {
        c::WORKSPACE_READ => "Read channels, threads, messages and files",
        c::WORKSPACE_WRITE => "Create and change channels, threads and tasks",
        c::MESSAGE_POST => "Post messages and replies",
        c::THREAD_TRANSITION => "Move tasks through review and done",
        c::ARTIFACT_UPLOAD => "Upload files",
        c::SEARCH_QUERY => "Search the workspace",
        c::EVENT_SUBSCRIBE => "Follow live updates",
        other => other,
    }
}

/// The capability list a person reads before deciding: a sentence for each,
/// with the capability's name beside it.
pub(super) fn capability_list(scope: &[String]) -> String {
    let rows: String = scope
        .iter()
        .map(|s| {
            format!(
                "<li><span>{}</span><code>{}</code></li>",
                html_escape(describe(s)),
                html_escape(s)
            )
        })
        .collect();
    format!(r#"<ul class="caps">{rows}</ul>"#)
}

pub(super) fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_csp_admits_the_stylesheet_by_its_hash_and_nothing_else_inline() {
        let csp = csp();
        let expected = STANDARD.encode(Sha256::digest(CSS.as_bytes()));
        assert!(csp.contains(&format!("style-src 'self' 'sha256-{expected}'")));
        assert!(!csp.contains("unsafe-inline"));
        assert!(csp.contains("form-action 'self'"));
    }

    #[test]
    fn the_page_carries_the_stylesheet_byte_for_byte() {
        // The hash is of CSS exactly; any change between the constant and
        // the element would make browsers drop the style.
        let page = render("t", "", "");
        assert!(page.contains(&format!("<style>{CSS}</style>")));
    }

    #[test]
    fn a_capability_list_escapes_what_it_shows() {
        let list = capability_list(&["<x>".to_string(), "workspace:read".to_string()]);
        assert!(list.contains("<code>&lt;x&gt;</code>"));
        assert!(list.contains("Read channels"));
        assert!(!list.contains("<x>"));
    }
}
