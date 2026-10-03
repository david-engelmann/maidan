//! Guard against "function called but never defined" in the `/ui` console JS.
//! The `/ui` is vanilla HTML/JS with no browser in CI, so a
//! reference-to-undefined-function bug (which is exactly what broke the write
//! path — `apiWritePath`/`requireAuthForWrite` were called but never defined)
//! otherwise sails through `cargo test`. This is a dependency-free static
//! check: every *bare* call `ident(` must resolve to a local definition, a
//! function parameter, or a known JS/DOM global.

const HTML: &str = include_str!("../static/index.html");
const UI_CSS: &str = include_str!("../static/ui/board.css");

fn page() -> &'static str {
    static CELL: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    CELL.get_or_init(|| format!("{HTML}\n{UI_CSS}")).as_str()
}

/// The board program with module import and export lines removed, so the
/// existing contract can read it as one script. The browser loads the modules.
fn script(_html: &str) -> &'static str {
    static CELL: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    CELL.get_or_init(|| {
        const PARTS: &[&str] = &[
            include_str!("../static/ui/state.js"),
            include_str!("../static/ui/client.js"),
            include_str!("../static/ui/api.js"),
            include_str!("../static/ui/feedback.js"),
            include_str!("../static/ui/people.js"),
            include_str!("../static/ui/session.js"),
            include_str!("../static/ui/board.js"),
            include_str!("../static/ui/needs.js"),
            include_str!("../static/ui/thread.js"),
            include_str!("../static/ui/dm.js"),
            include_str!("../static/ui/artifacts.js"),
            include_str!("../static/ui/tools.js"),
            include_str!("../static/ui/palette.js"),
            include_str!("../static/ui/realtime.js"),
            include_str!("../static/ui/main.js"),
        ];
        let mut out = String::new();
        for part in PARTS {
            for line in part.lines() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("import ") || trimmed.starts_with("export {") {
                    continue;
                }
                if let Some(rest) = trimmed.strip_prefix("export ") {
                    let indent = &line[..line.len() - trimmed.len()];
                    out.push_str(indent);
                    out.push_str(rest);
                    out.push('\n');
                    continue;
                }
                out.push_str(line);
                out.push('\n');
            }
        }
        out
    })
    .as_str()
}

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$'
}

/// The identifier ending at byte `idx` (exclusive), scanning left.
fn ident_ending_at(s: &str, idx: usize) -> Option<(usize, &str)> {
    let bytes = s.as_bytes();
    let mut i = idx;
    while i > 0 && is_ident(bytes[i - 1] as char) {
        i -= 1;
    }
    if i == idx {
        None
    } else {
        Some((i, &s[i..idx]))
    }
}

/// The identifier starting at byte `idx`, scanning right.
fn ident_starting_at(s: &str, idx: usize) -> Option<&str> {
    let bytes = s.as_bytes();
    let mut j = idx;
    while j < s.len() && is_ident(bytes[j] as char) {
        j += 1;
    }
    if j == idx {
        None
    } else {
        Some(&s[idx..j])
    }
}

/// Names introduced by `function NAME`, `const/let/var NAME`.
fn collect_defined(s: &str, out: &mut std::collections::HashSet<String>) {
    for kw in ["function ", "const ", "let ", "var "] {
        let mut from = 0;
        while let Some(rel) = s[from..].find(kw) {
            let after = from + rel + kw.len();
            // whole-word keyword (preceded by non-ident)
            let pre_ok = from + rel == 0 || !is_ident(s.as_bytes()[from + rel - 1] as char);
            if pre_ok {
                if let Some(name) = ident_starting_at(s, after) {
                    out.insert(name.to_string());
                }
            }
            from = after;
        }
    }
}

/// Best-effort function-parameter names: identifiers inside a `(...)` that is
/// immediately followed by `=>`, plus a single `x =>` param. Conservative — it
/// over-collects (treats any such ident as defined), which only *weakens* the
/// check, never produces a false failure.
fn collect_params(s: &str, out: &mut std::collections::HashSet<String>) {
    let bytes = s.as_bytes();
    let mut i = 0;
    while let Some(rel) = s[i..].find("=>") {
        let arrow = i + rel;
        // skip whitespace left of =>
        let mut k = arrow;
        while k > 0 && (bytes[k - 1] as char).is_whitespace() {
            k -= 1;
        }
        if k > 0 && bytes[k - 1] == b')' {
            // (... ) => : collect idents inside the matching paren group
            let close = k - 1;
            let mut depth = 1i32;
            let mut p = close;
            while p > 0 && depth > 0 {
                p -= 1;
                match bytes[p] {
                    b')' => depth += 1,
                    b'(' => depth -= 1,
                    _ => {}
                }
            }
            let inner = &s[p + 1..close];
            for tok in inner.split(|c: char| !is_ident(c)) {
                if !tok.is_empty() && !tok.chars().next().unwrap().is_ascii_digit() {
                    out.insert(tok.to_string());
                }
            }
        } else if let Some((_, name)) = ident_ending_at(s, k) {
            // x => : single bare param
            out.insert(name.to_string());
        }
        i = arrow + 2;
    }
}

/// JS keywords + globals that can legitimately appear as a bare `ident(`.
const ALLOWED: &[&str] = &[
    // keywords that precede `(`
    "if",
    "for",
    "while",
    "switch",
    "catch",
    "return",
    "function",
    "typeof",
    "await",
    "do",
    // JS globals / builtins
    "fetch",
    "alert",
    "confirm",
    "prompt",
    "setTimeout",
    "setInterval",
    "clearTimeout",
    "clearInterval",
    "parseInt",
    "parseFloat",
    "isNaN",
    "isFinite",
    "encodeURIComponent",
    "decodeURIComponent",
    "btoa",
    "atob",
    "String",
    "Number",
    "Boolean",
    "Array",
    "Object",
    "Promise",
    "Map",
    "Set",
    "Date",
    "Error",
    "RegExp",
    "Symbol",
    "JSON",
    "Math",
    "structuredClone",
    "queueMicrotask",
    "requestAnimationFrame",
    "WebSocket",
    "URL",
    "URLSearchParams",
    "Blob",
    "FormData",
    "TextEncoder",
    "TextDecoder",
    "Uint8Array",
    // CSS functions inside Web Animations keyframe strings
    "scale",
    "translate",
    "bezier",
];

#[test]
fn ui_js_has_no_undefined_bare_function_calls() {
    let s = script(HTML);
    let mut known = std::collections::HashSet::new();
    collect_defined(s, &mut known);
    collect_params(s, &mut known);
    for a in ALLOWED {
        known.insert((*a).to_string());
    }

    let bytes = s.as_bytes();
    let mut unresolved: Vec<String> = Vec::new();
    for (idx, b) in bytes.iter().enumerate() {
        if *b != b'(' {
            continue;
        }
        let Some((start, name)) = ident_ending_at(s, idx) else {
            continue;
        };
        // skip method calls (`foo.bar(`) and property access — preceded by `.`
        if start > 0 && bytes[start - 1] == b'.' {
            continue;
        }
        // skip pure-numeric (won't happen for idents) already handled by ident scan
        if !known.contains(name) && !unresolved.contains(&name.to_string()) {
            unresolved.push(name.to_string());
        }
    }

    assert!(
        unresolved.is_empty(),
        "/ui index.html calls these as functions but they are neither defined, \
         a parameter, nor a known global — likely a typo or a removed helper \
         (CI has no browser to catch this at runtime): {unresolved:?}"
    );
}

/// The live thread view wires WS event frames into the message list. No browser
/// in CI, so guard the wiring statically — the helper must be defined, invoked,
/// and driven by the thread-content kind set + the open-thread predicate in the
/// WS handler.
#[test]
fn ui_js_wires_live_thread_refresh() {
    let s = script(HTML);
    assert!(
        s.contains("function scheduleLiveRefresh("),
        "scheduleLiveRefresh must be defined"
    );
    assert!(
        s.contains("scheduleLiveRefresh()"),
        "scheduleLiveRefresh must be invoked (dead helper otherwise)"
    );
    assert!(
        s.contains("liveFrameTargetsOpenThread(v)"),
        "the WS handler must gate the refresh on the open thread"
    );
    for kind in [
        "message_posted",
        "message_edited",
        "reaction_added",
        "message_pinned",
    ] {
        assert!(s.contains(kind), "THREAD_CONTENT_KINDS must include {kind}");
    }
}

/// The Session tab's capability card. No browser in the required jobs, so guard
/// the wiring statically — the loader must be defined, wired to the tab switch,
/// and read the real grant from `/me`.
#[test]
fn ui_js_wires_session_capability_card() {
    let s = script(HTML);
    assert!(
        s.contains("function loadSession("),
        "loadSession must be defined"
    );
    assert!(
        s.contains("uiReadPath(\"/me\")"),
        "the card must read the real grant from GET /me"
    );
    assert!(
        s.contains("known_capabilities"),
        "\"can't\" must be computed from the known-capability vocabulary"
    );
    assert!(
        page().contains("data-tab=\"session\""),
        "the Session tab button must exist"
    );
    assert!(
        page().contains("id=\"panel-session\""),
        "the Session panel must exist"
    );
    assert!(
        s.contains("\"session\") loadSession()"),
        "the tab switch must invoke loadSession()"
    );
}

/// Session-chrome badges on the thread list and board. Guard the mapping's
/// wiring statically — the pure classifier must be defined, invoked by
/// loadThreads, read the real FSM state (`in_review`) and claim, and cover every
/// chrome state. There is no catch-all "idle": an unclaimed thread is "open".
#[test]
fn ui_js_wires_session_chrome_badges() {
    let s = script(HTML);
    assert!(
        s.contains("function sessionChrome("),
        "sessionChrome must be defined"
    );
    assert!(
        s.contains("sessionChrome(th, gates[th.id])"),
        "loadThreads must classify each thread"
    );
    assert!(
        s.contains("function fetchPendingGatesByThread("),
        "the gate lookup must be defined"
    );
    assert!(
        s.contains("th.state === \"in_review\""),
        "the classifier must read the thread's FSM state, not only its claim"
    );
    assert!(
        !s.contains("chrome-idle"),
        "\"idle\" hid real state (in review, claimed); it must not come back"
    );
    for state in [
        "chrome-open",
        "chrome-claimed",
        "chrome-running",
        "chrome-in-review",
        "chrome-needs-input",
        "chrome-needs-approval",
        "chrome-done",
    ] {
        assert!(
            s.contains(state),
            "the chrome classifier must cover {state}"
        );
    }
}

/// Attenuation chrome — a minted token cannot widen the caller's grant. Guard
/// the wiring: the ceiling loader + the widening classifier are defined, the
/// classifier gates the mint handler, and the ceiling loads on the Tokens tab.
#[test]
fn ui_js_wires_attenuation_chrome() {
    let s = script(HTML);
    assert!(
        s.contains("function loadAttenuationCeiling("),
        "loadAttenuationCeiling must be defined"
    );
    assert!(
        s.contains("function capsExceedingGrant("),
        "capsExceedingGrant must be defined"
    );
    assert!(
        s.contains("capsExceedingGrant(caps)"),
        "the mint handler must pre-flight the widening set"
    );
    assert!(
        s.contains("\"tokens\") loadAttenuationCeiling()"),
        "the Tokens tab must load the attenuation ceiling"
    );
    assert!(
        page().contains("id=\"attenuation-warning\""),
        "the attenuation warning element must exist"
    );
}

/// WCAG-AA keyboard operability. Guard the ARIA tablist wiring + the skip link
/// statically — the browser behaviour is covered by the Playwright spec, but
/// these must be present for the a11y contract to hold.
#[test]
fn ui_js_wires_wcag_tablist_and_skip_link() {
    let s = script(HTML);
    assert!(
        page().contains("class=\"skip-link\"") && page().contains("id=\"main-content\""),
        "a skip link must target the main content (WCAG 2.4.1)"
    );
    assert!(
        s.contains("function initTablist(") && s.contains("initTablist();"),
        "initTablist must be defined and invoked"
    );
    assert!(
        s.contains("\"role\", \"tab\"") && s.contains("\"role\", \"tabpanel\""),
        "tabs and panels must carry ARIA roles"
    );
    for key in ["ArrowRight", "ArrowLeft", "Home", "End"] {
        assert!(
            s.contains(key),
            "the tablist must support {key} keyboard navigation"
        );
    }
}

/// The Work tab. No browser in the required jobs, so guard the wiring
/// statically — the loaders are defined + invoked, the tab switch calls
/// `loadWork`, and the panel + channel selector exist.
#[test]
fn ui_js_wires_work_tab() {
    let s = script(HTML);
    for f in [
        "async function loadWork(",
        "async function loadWorkChannels(",
        "async function loadWorkDepth(",
        "async function loadWorkThreads(",
        "async function showWorkThread(",
        "async function loadWorkSchedules(",
    ] {
        assert!(s.contains(f), "the Work tab must define {f}");
    }
    assert!(
        s.contains("=== \"work\") loadWork()"),
        "the tab switch must call loadWork() for the Work tab"
    );
    assert!(
        s.contains("loadWorkDepth()") && s.contains("loadWorkThreads()"),
        "the channel selector must load depth + threads"
    );
    assert!(
        page().contains("id=\"panel-work\"") && page().contains("id=\"work-channel\""),
        "the Work panel + channel selector must exist"
    );
    assert!(
        page().contains("data-tab=\"work\""),
        "the Work tab button must exist"
    );
}

/// The Prefs console. Static guard — the loaders + mutators are defined, the
/// tab switch calls `loadPrefs`, and the panel exists.
#[test]
fn ui_js_wires_prefs_tab() {
    let s = script(HTML);
    for f in [
        "async function loadPrefs(",
        "async function loadPrefsFollows(",
        "async function setPrefsDeliveryMode(",
        "async function setPrefsMute(",
        "async function followTarget(",
        "async function unfollowTarget(",
    ] {
        assert!(s.contains(f), "the Prefs tab must define {f}");
    }
    assert!(
        s.contains("=== \"prefs\") loadPrefs()"),
        "the tab switch must call loadPrefs() for the Prefs tab"
    );
    assert!(
        page().contains("id=\"panel-prefs\"") && page().contains("data-tab=\"prefs\""),
        "the Prefs panel + tab button must exist"
    );
}

/// The looking-glass explorer. Static guard — the explorers are defined, the
/// tab switch calls `loadGlass`, and the panel exists.
#[test]
fn ui_js_wires_looking_glass_tab() {
    let s = script(HTML);
    for f in [
        "function loadGlass(",
        "async function glassEventsByKind(",
        "async function glassThread(",
        "async function glassArtifact(",
        "async function glassPeers(",
    ] {
        assert!(s.contains(f), "the looking glass must define {f}");
    }
    assert!(
        s.contains("=== \"glass\") loadGlass()"),
        "the tab switch must call loadGlass() for the looking-glass tab"
    );
    assert!(
        page().contains("id=\"panel-glass\"") && page().contains("data-tab=\"glass\""),
        "the looking-glass panel + tab button must exist"
    );
}

/// The waiting-on-you inbox in the Work tab. Static guard — the loader is
/// defined + invoked from loadWork, and the section exists.
#[test]
fn ui_js_wires_waiting_inbox() {
    let s = script(HTML);
    assert!(
        s.contains("async function loadWaiting("),
        "loadWaiting must be defined"
    );
    assert!(
        s.contains("await loadWaiting()"),
        "loadWork must invoke loadWaiting"
    );
    assert!(
        s.contains("/waiting?sla_secs="),
        "loadWaiting must call the waiting inbox with an sla_secs"
    );
    assert!(
        page().contains("id=\"waiting-list\"") && page().contains("id=\"waiting-sla\""),
        "the waiting-on-you section must exist"
    );
}

#[test]
fn ui_js_wires_honest_async_states_and_live_approvals() {
    let s = script(HTML);
    for helper in [
        "function setLoading(",
        "function clearLoading(",
        "async function responseError(",
        "function renderState(",
    ] {
        assert!(s.contains(helper), "the UI must define {helper}");
    }
    assert!(
        s.contains("problem.detail || problem.error || problem.message || problem.title"),
        "API problem details must survive into the visible error"
    );
    assert!(
        s.contains("panel.classList.contains(\"active\")")
            && s.contains("loadApprovals(false)")
            && s.contains("kind === \"approval_requested\""),
        "the visible approvals queue must poll and react to live gate events"
    );
    for message in [
        "No channels yet. Create one above",
        "No tasks in this channel yet. Add one above",
        "No messages yet. Start the conversation below",
        "You're caught up — no pending approval gates",
    ] {
        assert!(
            s.contains(message),
            "the UI must retain the actionable empty state: {message}"
        );
    }
}

#[test]
fn ui_uses_locked_brand_mark_and_palette() {
    for color in ["#f7f5f0", "#14532d", "#4ade80", "#b45309", "#232327"] {
        assert!(
            page().contains(color),
            "the UI must retain brand color {color}"
        );
    }
    assert!(
        page().contains("<svg class=\"brand-mark\" viewBox=\"0 0 64 64\"")
            && page().contains("M38.95,32.85L48.07,37.53L59.65,35.70")
            && page().contains("M37.52,27.69L47.28,24.55L51.08,17.60"),
        "the /ui header must retain the locked Sweep Reach mark"
    );
}

/// People are shown by name. Every place that renders a member resolves it
/// through the workspace member directory; the raw id survives only as a
/// tooltip.
#[test]
fn ui_js_renders_members_by_display_name() {
    let s = script(HTML);
    for f in [
        "async function loadMembers(",
        "function memberName(",
        "function personEl(",
    ] {
        assert!(s.contains(f), "the UI must define {f}");
    }
    assert!(
        s.contains("m.display_name || m.handle"),
        "names come from display_name, falling back to the handle"
    );
    for raw in [
        "`${m.id} · ${m.author_id}`",
        "`${m.author_id}: ${m.body}`",
        "`${m.member_id} · ${m.status}",
    ] {
        assert!(
            !s.contains(raw),
            "a raw member id is rendered as text again: {raw}"
        );
    }
}

/// The channel renders as a live board, and the Live bar stays collapsed until
/// the socket is up.
#[test]
fn ui_js_wires_live_board_and_collapsed_live_bar() {
    let s = script(HTML);
    assert!(
        s.contains("function renderBoard("),
        "renderBoard must be defined"
    );
    assert!(
        s.contains("renderBoard(threads, gates)"),
        "loadThreads must render the board"
    );
    assert!(
        s.contains("THREAD_BOARD_KINDS.has(kind)) scheduleBoardRefresh()"),
        "thread/claim events on the socket must refresh the board"
    );
    assert!(
        page().contains("<pre id=\"live-feed\" hidden>"),
        "the raw live feed starts hidden"
    );
    assert!(
        s.contains("classList.toggle(\"connected\", cls === \"connected\")"),
        "the Live bar expands only once connected"
    );
}

#[test]
fn ui_js_thread_header_ignores_stale_overlapping_renders() {
    // A quiet board refresh can re-render the thread header while an earlier
    // render (from a card click) is still awaiting /result and /review-status.
    // Each render takes a generation token, and a stale one must not append facts.
    let js = script(HTML);
    let start = js
        .find("async function renderThreadHeader()")
        .expect("renderThreadHeader is defined");
    let body = &js[start
        ..start
            + js[start..]
                .find("function selectThread")
                .expect("selectThread")];
    assert!(
        body.contains("const gen = ++headerGen;"),
        "each render takes a generation"
    );
    assert!(
        body.matches("gen !== headerGen").count() >= 2,
        "results from a stale render are dropped after each await"
    );
    assert!(
        !body.contains("tid === selectedThreadId"),
        "the thread-id guard alone lets two renders of the same thread both append"
    );
}

#[test]
fn ui_js_puts_the_decisions_agents_wait_on_first() {
    let s = script(HTML);
    assert!(
        page().contains("id=\"needs-you\"") && page().contains("id=\"needs-you-list\""),
        "the Needs you queue must exist"
    );
    let needs = HTML.find("id=\"needs-you\"").expect("needs-you");
    let board = HTML.find("id=\"board-panel\"").expect("board-panel");
    assert!(needs < board, "Needs you sits above the board");
    assert!(
        s.contains("uiReadPath(`/members/${me}/waiting`)")
            && s.contains("new Set([\"review_request\", \"open_gate\"])"),
        "the queue reads the waiting inbox and keeps the decisions: reviews and gates"
    );
    assert!(
        s.contains("apiWritePath(`/threads/${tid}/reviews`)")
            && s.contains("{ action: \"close\" }"),
        "approve, request changes and close go through the review and FSM routes"
    );
    assert!(
        s.contains("document.title = n > 0 ? `(${n}) Maidan` : \"Maidan\""),
        "the waiting count reaches the tab title"
    );
    assert!(
        s.contains("renderNeedsYou") && s.contains("loadNeedsYou();"),
        "the queue refreshes with the board"
    );
}

#[test]
fn ui_js_renders_results_as_fields_and_errors_as_sentences() {
    let s = script(HTML);
    assert!(
        s.contains("function renderResult(")
            && s.contains("fact(\"result \", renderResult(r.result))"),
        "the thread header shows a result as fields, not JSON.stringify"
    );
    assert!(
        !s.contains("code.textContent = JSON.stringify(r.result)"),
        "the raw-JSON result rendering is gone"
    );
    assert!(
        s.contains("function humanError(") && s.contains("Mint a token with it in Tokens"),
        "a 403 names the missing capability and the fix"
    );
    assert!(
        s.contains("const said = humanError(res.status, detail);"),
        "every responseError leads with the plain sentence"
    );
}

#[test]
fn ui_js_drops_thread_responses_for_a_channel_no_longer_selected() {
    let s = script(HTML);
    assert!(
        s.contains("const stale = () => selectedChannelId !== channelId || gen !== threadLoadGen;")
            && s.contains("if (stale()) return;\n          renderBoard(threads, gates);"),
        "loadThreads renders only for the channel it was asked for"
    );
    assert!(
        s.contains("li[data-id=\"${CSS.escape(remembered)}\"]"),
        "the remembered channel id is escaped before it goes into a selector"
    );
}

#[test]
fn ui_js_loads_every_page_of_a_channel_and_only_the_newest_load_paints() {
    let s = script(HTML);
    let start = s.find("async function loadThreads").expect("loadThreads");
    // A fixed byte window can land inside a multibyte character (the
    // ellipsis in a comment, for example), so the end walks back to a
    // character boundary.
    let end = (start + 3000).min(s.len());
    let end = (0..=end)
        .rev()
        .find(|i| s.is_char_boundary(*i))
        .unwrap_or(start);
    let body = &s[start..end];
    assert!(
        body.contains("const gen = ++threadLoadGen;"),
        "each load takes a generation, so an older load for the same channel is dropped"
    );
    assert!(
        body.contains("q.set(\"cursor\", cursor)")
            && body.contains("if (batch.length < pageSize) break;")
            && body.contains("const next = batch[batch.length - 1].id;")
            && body.contains("cursor = next;"),
        "the board follows the keyset cursor until a short page instead of stopping at one page"
    );
}

#[test]
fn ui_js_socket_presence_uses_the_member_it_authenticates_as() {
    let s = script(HTML);
    assert!(
        s.contains("const presenceId = authorId();")
            && s.contains("if (presenceId) frame.member_id = presenceId;"),
        "presence goes to the bearer's member when a token is set, matching who the socket authenticates as"
    );
    assert!(
        !s.contains("if (sessionMemberId) frame.member_id = sessionMemberId;"),
        "the session member is not sent alongside another member's token"
    );
}

#[test]
fn ui_js_keeps_a_needs_you_row_until_its_decision_is_recorded() {
    let s = script(HTML);
    assert!(
        s.contains("return { ok: true };") && s.contains("const out = await answerGate(item.gate_id, action, v.request_state);\n              if (!out.ok) {"),
        "a gate row leaves only when the answer was recorded, and says why otherwise"
    );
    assert!(
        s.contains("if (send.disabled) return;"),
        "Enter cannot send the same change request twice"
    );
}

#[test]
fn ui_js_review_and_close_report_network_failures_instead_of_rejecting() {
    let s = script(HTML);
    for (func, lead) in [
        (
            "async function submitReview",
            "Review not recorded: could not reach the server",
        ),
        (
            "async function closeThread",
            "Not closed: could not reach the server",
        ),
    ] {
        let start = s.find(func).expect(func);
        let end = (start + 900).min(s.len());
        let end = (0..=end)
            .rev()
            .find(|i| s.is_char_boundary(*i))
            .unwrap_or(start);
        let body = &s[start..end];
        assert!(
            body.contains("try {") && body.contains("} catch (e) {") && body.contains(lead),
            "{func} turns a thrown fetch into {{ ok: false, why }} so its row re-enables and says why"
        );
    }
}

#[test]
fn ui_js_shows_the_team_and_moves_cards_between_lanes() {
    let html = HTML;
    let s = script(HTML);
    assert!(
        html.contains("<div id=\"team\"") && s.contains("renderTeam(threads);"),
        "the board renders a team strip with every refresh"
    );
    assert!(
        s.contains("const LIVE_MS = 120000;")
            && s.contains("markSeenFromFrame(v);\n            refreshTeamSoon();"),
        "live means seen on the socket in the last two minutes, fed by event frames"
    );
    assert!(
        s.contains("for (const k of ACTOR_KEYS) if (!skip.includes(k) && typeof v[k] === \"string\") markSeen(v[k]);")
            && !s.contains("(v && v.payload) || {}"),
        "event frames are flat: actor fields are read from the top level"
    );
    assert!(
        s.contains("m.status === \"online\" && markSeen(m.member_id)"),
        "the presence snapshot marks online members live"
    );
    assert!(
        s.contains("const before = boardRects();")
            && s.contains("if (!firstPaint) glideCards(before);"),
        "cards are measured before a re-render and played from there after (FLIP)"
    );
    assert!(
        s.contains("if (reduceMotion.matches || !before.size) return;")
            && page().contains("@media (prefers-reduced-motion: reduce)"),
        "nothing moves or pulses under prefers-reduced-motion"
    );
}

#[test]
fn ui_js_has_a_command_palette_and_connect_an_agent() {
    let html = HTML;
    let s = script(HTML);
    assert!(
        html.contains("<dialog id=\"palette\"") && s.contains("e.key.toLowerCase() === \"k\""),
        "Cmd/Ctrl+K opens a command palette"
    );
    assert!(
        s.contains("kind: \"Channel\"")
            && s.contains("kind: \"Task\"")
            && s.contains("kind: \"Tool\""),
        "the palette reaches channels, tasks and every tool tab"
    );
    assert!(
        html.contains("<dialog id=\"connect-dialog\"")
            && s.contains("`${base()}/mcp/streamable`")
            && s.contains("Bearer REPLACE_WITH_MAIDAN_TOKEN")
            && s.contains("claude mcp add --transport http maidan")
            && s.contains("cursor://anysphere.cursor-deeplink/mcp/install?name=maidan&config="),
        "Connect an agent builds MCP config for this server with a placeholder token"
    );
    assert!(
        !s.contains("Bearer ${token()}\" } }"),
        "the viewer token is never written into an agent snippet"
    );
    assert!(
        html.contains("id=\"board-onboard\"") && s.contains("function emptyChannelHelp("),
        "the empty board and an empty channel explain how work arrives"
    );
    assert!(
        html.contains("id=\"cx-create-agent\"")
            && s.contains("capability_set: WORKER_PRESET")
            && s.contains("const WORKER_PRESET = \"maidan.agent.worker\"")
            && html.contains("thread:transition"),
        "Connect an agent creates a member and mints the worker preset, which can transition"
    );
    assert!(
        !s.contains("document.getElementById(\"token\").value = minted.secret"),
        "minting an agent does not replace the browser token"
    );
}

#[test]
fn ui_js_palette_opens_the_next_review_by_its_summary() {
    let s = script(HTML);
    assert!(
        s.contains("(th && th.title) || next.summary || \"untitled\"")
            && s.contains("selectThread(next.thread_id, title);"),
        "the next-review action opens the thread and names it by the inbox summary"
    );
}

#[test]
fn ui_js_socket_retries_with_backoff_ignores_replaced_sockets_and_stops_on_refusal() {
    let s = script(HTML);
    assert!(
        s.contains("Math.min(30000, 1500 * 2 ** wsRetries)"),
        "a dropped socket retries with backoff instead of trying once"
    );
    assert!(
        s.matches("if (sock !== wsSocket) return;").count() >= 2,
        "a socket replaced by a newer one cannot clear or reconnect over it"
    );
    assert!(
        s.contains("const refused = ev && ev.code === 1008;"),
        "a policy refusal is not retried"
    );
    assert!(
        s.contains("setLiveFallback(true);") && s.contains("const LIVE_POLL_MS = 15000;"),
        "the board polls while the socket is down"
    );
    assert!(
        s.contains("window.addEventListener(\"online\", reconnectNowIfWanted);"),
        "coming back online reconnects at once"
    );
}

#[test]
fn ui_js_gate_rows_lead_with_the_question() {
    let s = script(HTML);
    assert!(
        s.contains("title.textContent = isGate ? item.summary : (th && th.title) || item.summary;"),
        "a gate row shows the question being approved, not only its task title"
    );
}

#[test]
fn ui_js_board_has_its_own_loading_and_error_states() {
    let s = script(HTML);
    assert!(
        s.contains("if (replacing) boardState(\"loading\");")
            && s.contains("if (replacing) boardState(\"error\", message);"),
        "a newly picked channel says it is loading, and says what failed"
    );
    assert!(
        s.contains("retry.textContent = \"Try again\";"),
        "the board error offers Try again"
    );
    assert!(
        !s.contains("Error: ${e}</li>"),
        "no raw exception text is written into HTML unescaped"
    );
}

#[test]
fn ui_js_missing_token_admin_does_not_send_you_back_to_tokens() {
    let s = script(HTML);
    assert!(
        s.contains("if (status === 403 && needs && needs[1] === \"token:admin\")"),
        "minting needs token:admin, so a token:admin refusal must not say mint one in Tokens"
    );
}

#[test]
fn ui_js_presence_skips_subjects_that_did_not_act() {
    let s = script(HTML);
    assert!(
        s.contains("claim_expired: [\"member_id\", \"assignee_id\"],")
            && s.contains("thread_assignment_changed: [\"assignee_id\", \"member_id\"],")
            && s.contains("if (!skip.includes(k) && typeof v[k] === \"string\") markSeen(v[k]);"),
        "a lapsed claim or a reassignment does not mark its subject live"
    );
}

#[test]
fn ui_js_thread_pages_have_no_cap_but_stop_on_a_stuck_cursor() {
    let s = script(HTML);
    assert!(!s.contains("page < 40"), "no 40-page cap on the board walk");
    assert!(
        s.contains("if (next === cursor) break;"),
        "a cursor that does not advance ends the walk"
    );
}

#[test]
fn ui_js_needs_you_clears_on_refusal_and_keeps_rows_in_use() {
    let s = script(HTML);
    assert!(
        s.contains("needsYou = [];\n            setAttention(0);"),
        "a refused inbox load clears the queue and the tab count"
    );
    assert!(
        s.contains("list.appendChild(keep.get(k) || needsYouRow(item));"),
        "a row in use survives a reload"
    );
    assert!(
        s.contains("document.getElementById(\"thread-actions\").replaceChildren();\n              headerGen++;"),
        "a channel switch drops the previous task's buttons and in-flight header"
    );
}

/// A blocking `alert()`, `confirm()` or `prompt()` stops the page and cannot
/// be styled, dismissed or read by a screen reader as part of it. The `/ui`
/// reports a user's mistake with `showError`, a toast in a `role="alert"`
/// region, and asks for confirmation inline (workspace purge: type the id and
/// tick the box).
#[test]
fn ui_js_reports_errors_without_blocking_dialogs() {
    let js = script(HTML);
    let bytes = js.as_bytes();
    for dialog in ["alert(", "confirm(", "prompt("] {
        let mut from = 0;
        while let Some(offset) = js[from..].find(dialog) {
            let at = from + offset;
            let called_bare =
                at == 0 || !(is_ident(bytes[at - 1] as char) || bytes[at - 1] == b'.');
            assert!(
                !called_bare,
                "a blocking {dialog}) at byte {at}: use showError or an inline confirmation"
            );
            from = at + dialog.len();
        }
    }
    assert!(
        page().contains(r#"<div id="toasts"></div>"#),
        "the toast region"
    );
}

/// The board is the one place a channel's threads are drawn. The sidebar used
/// to list the same threads from the same fetch, so each appeared twice in two
/// visual languages.
#[test]
fn ui_js_draws_a_channels_threads_once_on_the_board() {
    assert!(
        !page().contains("id=\"thread-list\""),
        "no second thread list"
    );
    let board_head = HTML
        .split("<div id=\"board-head\">")
        .nth(1)
        .and_then(|rest| rest.split("</div>\n          <div id=\"team\"").next())
        .expect("the board header");
    assert!(
        board_head.contains("id=\"new-thread-title\"")
            && board_head.contains("id=\"create-thread\""),
        "a new task is created from the board"
    );
}

/// A blank page connects from a first-run card, not from paste fields in the
/// header, and offers an identity provider only when the server's discovery
/// document says it has one.
#[test]
fn ui_js_first_run_offers_only_the_sign_in_paths_the_server_has() {
    let header = HTML
        .split("<header>")
        .nth(1)
        .and_then(|rest| rest.split("</header>").next())
        .expect("the header");
    assert!(
        !header.contains("id=\"conn-fields\"") && !header.contains("id=\"login\""),
        "the header starts without the connection inputs"
    );
    let first_run = HTML
        .split("id=\"first-run\"")
        .nth(1)
        .and_then(|rest| rest.split("</section>").next())
        .expect("the first-run card");
    assert!(
        first_run.contains("id=\"conn-fields\"") && first_run.contains("id=\"login\""),
        "the first-run card holds the inputs and the sign-in button"
    );
    assert!(
        first_run.contains("exchanges it for a session and does not keep the token"),
        "the card says the browser does not keep the token"
    );
    let s = script(HTML);
    assert!(
        s.contains("/.well-known/maidan.json") && s.contains("oidcLoginPath"),
        "the identity-provider button follows the discovery document"
    );
    assert!(
        s.contains("localStorage.removeItem(tokenKey)"),
        "Sign out forgets the token"
    );
}

/// A token acts as exactly one member (Cluster 411), so the Session tab must
/// not say a bearer acts as any member, and it rotates the token it runs on
/// through `POST /tokens/{id}/rotate`, the id coming from `/me`.
#[test]
fn ui_js_says_what_a_token_is_and_rotates_it() {
    assert!(
        !page().contains("acts as any member"),
        "the pre-411 impersonation wording is gone"
    );
    let s = script(HTML);
    assert!(
        s.contains("/rotate`") && s.contains("me.token_id"),
        "rotation uses the rotate route and the token id /me returns"
    );
    assert!(
        page().contains("id=\"rotate-own-token\"") && page().contains("id=\"rotate-token\""),
        "the Session tab and the Tokens tab both offer rotation"
    );
}

/// The source of `function NAME(` up to its closing brace at the same indent.
fn function_body<'a>(js: &'a str, name: &str) -> &'a str {
    let start = js
        .find(&format!("function {name}("))
        .unwrap_or_else(|| panic!("function {name}"));
    let end = js[start..]
        .find("\n      }\n")
        .unwrap_or_else(|| panic!("end of {name}"));
    &js[start..start + end]
}

/// An attachment shows its name and, for a raster image, the image. The name
/// is hostile data from another member, so it is only ever text; the bytes
/// come through `fetch` with the viewer's credentials, so no token is ever in
/// a URL; and an SVG, which can carry script, is never drawn from a blob URL
/// that would run it in this origin.
#[test]
fn ui_js_previews_attachments_as_text_names_and_fetched_images() {
    let js = script(HTML);
    assert!(
        js.contains(
            r#"const INLINE_IMAGE_TYPES = new Set(["image/png", "image/jpeg", "image/gif", "image/webp"]);"#
        ),
        "the inline allowlist is the server's, without SVG"
    );

    let card = function_body(js, "artifactCard");
    assert!(
        !card.contains("innerHTML"),
        "the card builds nodes, not markup"
    );
    assert!(
        card.contains("name.textContent = `📎 ${filename"),
        "the filename is set as text"
    );
    assert!(
        card.contains("meta.filename") && card.contains("artifactMeta(sha)"),
        "the name comes from the workspace's metadata, not the message"
    );
    assert!(
        card.contains("img.src = artifactObjectUrl("),
        "an image loads from a fetched blob, never a URL carrying credentials"
    );
    assert!(
        card.contains("INLINE_IMAGE_TYPES.has(type)"),
        "only allowlisted types become images"
    );

    let blob = function_body(js, "artifactBlob");
    assert!(
        blob.contains("headers: { ...headers()") && blob.contains(r#"credentials: "include""#),
        "bytes ride the bearer header or the session cookie"
    );
    assert!(
        blob.contains("new Blob([await res.arrayBuffer()], { type })"),
        "the blob's type is the one the page chose"
    );
    for leak in ["?token=", "&token=", "access_token=", "token()}`"] {
        assert!(!js.contains(leak), "a token in a URL: {leak}");
    }

    let upload = function_body(js, "uploadArtifact");
    assert!(
        upload.contains(r#"params.set("filename", blob.name)"#),
        "an upload sends the file's name"
    );
    assert!(
        !js.contains("sha.slice(0, 16)") && !js.contains("Attached artifact"),
        "the SHA-prefix link and message are gone"
    );
}

#[test]
fn ui_js_opens_the_only_channel() {
    let s = script(HTML);
    assert!(
        s.contains("else if (channels.length === 1) list.querySelector(\"li[data-id]\").click();"),
        "a workspace with one channel opens it instead of asking the viewer to pick"
    );
    assert!(
        s.contains("if (!selectedChannelId)") && s.contains("if (again) again.click();"),
        "a remembered channel still wins, and a channel already open is not reloaded"
    );
}

#[test]
fn ui_js_shows_a_refused_close_and_leads_with_the_latest_review() {
    let s = script(HTML);
    let html = HTML;
    assert!(
        html.contains("id=\"board-refusal\"")
            && s.contains("notice !== \"transition_refused\"")
            && s.contains("Close refused"),
        "the board shows a close the server refused"
    );
    assert!(
        s.contains("String(b.since).localeCompare(String(a.since))")
            && s.contains("The latest request is first."),
        "Needs you puts the latest review first when more than one is waiting"
    );
}

/// The live control is Connect WS in the Live toolbar, not an Events tab.
#[test]
fn ui_js_names_the_live_connect_control() {
    assert!(
        !page().contains("Events tab") && !script(HTML).contains("Events tab"),
        "nothing still tells the reader to connect from an Events tab"
    );
    assert!(
        !page().contains("Events (HTTP)</strong>"),
        "the presence copy no longer points at the Events (HTTP) tab"
    );
    assert!(
        page().contains(
            "title=\"This thread updates live from the WebSocket. Press Connect WS in the Live toolbar.\""
        ),
        "the live mark names Connect WS"
    );
    assert!(
        page().contains("Press <strong>Connect WS</strong> in the Live"),
        "the roster names Connect WS in the Live toolbar"
    );
    let presence = function_body(script(HTML), "setPresence");
    assert!(
        presence.contains("Press Connect WS in the Live toolbar first"),
        "setting presence names Connect WS"
    );
    assert!(
        page().contains("id=\"ws-connect\""),
        "the named control is the connect button"
    );
}

/// The DM header names the other person. The list already does; the selection does too.
#[test]
fn ui_js_selects_a_dm_by_member_name() {
    let body = function_body(script(HTML), "selectDm");
    assert!(
        body.contains("textContent = `with ${memberName(dmOther(c))}`"),
        "the selection header uses the member name"
    );
    assert!(
        !body.contains("textContent = `with ${dmOther(c)}`"),
        "the selection header is not the raw member id"
    );
}

/// Item 2a already says the browser exchanges a token for a session and does
/// not keep it. What is left is the plaintext localStorage residual: the
/// workspace id and the live resume token.
#[test]
fn ui_js_warns_that_local_storage_is_plaintext() {
    let first_run = HTML
        .split("id=\"first-run\"")
        .nth(1)
        .and_then(|rest| rest.split("</section>").next())
        .expect("the first-run card");
    assert!(
        first_run.contains("exchanges it for a session and does not keep the token"),
        "the retention sentence stays"
    );
    assert!(
        first_run.contains("id=\"storage-warning\"")
            && first_run.contains("localStorage as plaintext"),
        "the login card states the plaintext localStorage residual"
    );
    assert!(
        !first_run.contains("keeps the token until you sign out"),
        "the token field no longer says the browser keeps the token"
    );
    let tokens = HTML
        .split("id=\"panel-tokens\"")
        .nth(1)
        .and_then(|rest| rest.split("</section>").next())
        .expect("the Tokens tab");
    assert!(
        tokens.contains("id=\"token-storage-warning\"") && tokens.contains("as plaintext"),
        "the Tokens tab states the same residual"
    );
}

/// A refused close still lands on the card, from one channel events read.
/// Not one message-history fetch per in_review thread. The strip stays hidden.
#[test]
fn ui_js_hydrates_refusals_from_one_notices_read() {
    let body = function_body(script(HTML), "hydrateRefusals");
    assert!(
        !body.contains("/messages"),
        "hydration does not fetch thread history"
    );
    assert!(
        body.contains("apiReadPath(`/workspaces/${wid()}/events?${q}`)")
            && body.contains("types: \"message_posted\""),
        "hydration is one message_posted events read for the channel"
    );
    assert!(
        body.contains("fetchPendingGatesByThread")
            || script(HTML).contains("const gates = await fetchPendingGatesByThread()"),
        "the board still has the one gates read"
    );
    assert!(
        body.contains("meta.notice !== \"transition_refused\"")
            && body.contains("rememberRefusal("),
        "a transition_refused notice is still remembered"
    );
    let paint = function_body(script(HTML), "paintRefusal");
    assert!(
        paint.contains("banner.hidden = true"),
        "the refusal strip stays hidden"
    );
}

/// The first screen is the board. Live controls sit in a closed menu, a
/// refused subscribe is a status line, an empty Needs you queue is one
/// sentence, and a decision row has a single filled button.
#[test]
fn ui_js_keeps_the_first_screen_quiet() {
    assert!(
        page().contains("id=\"live-more\"") && page().contains("id=\"needs-you-quiet\""),
        "the live menu and the empty-queue line exist"
    );
    assert!(
        !page().contains("class=\"primary\">Connect WS"),
        "connecting the socket is not the page's primary button"
    );
    assert!(
        !page().contains("Connect to update the board"),
        "the live hint no longer leads the first screen"
    );
    assert!(
        page().contains("Nothing is waiting on you."),
        "an empty queue is one sentence"
    );
    let js = script(HTML);
    assert!(
        js.contains("document.getElementById(\"needs-you-head\").hidden = empty;"),
        "the card head is hidden when nothing is waiting"
    );
    assert!(
        js.contains("changes.className = \"ghost\"")
            && js.contains("[\"decline\", \"Decline\", \"ghost\"]"),
        "the second action on a decision row is not a filled button"
    );
    assert!(
        js.contains("el.title = text || \"\";"),
        "a long refusal stays available without becoming a banner"
    );
}

/// More tools is closed on first paint. The command palette still opens a tab.
#[test]
fn ui_js_more_tools_starts_closed() {
    assert!(
        page().contains("<details id=\"tools\">"),
        "More tools is on the page"
    );
    assert!(
        !page().contains("<details id=\"tools\" open"),
        "More tools starts closed; the palette opens it"
    );
    let body = function_body(script(HTML), "openTool");
    assert!(
        body.contains("tools.open = true") && body.contains("tab.click()"),
        "openTool still opens More tools and selects the tab"
    );
}

/// The thread panel, including the composer and Post, stays off the first
/// screen until a card is open. Switching channels hides it again.
#[test]
fn ui_js_hides_the_composer_until_a_card_is_open() {
    assert!(
        page().contains("<section id=\"collab-panel\" hidden>"),
        "the thread panel starts hidden"
    );
    assert!(
        page().contains("#collab-panel[hidden] { display: none; }"),
        "a hidden thread panel stays off the first screen"
    );
    let js = script(HTML);
    let sync = function_body(js, "syncCollabPanel");
    assert!(
        sync.contains("getElementById(\"collab-panel\").hidden = !selectedThreadId"),
        "the panel is hidden while no card is open"
    );
    let select = function_body(js, "selectThread");
    assert!(
        select.contains("syncCollabPanel()"),
        "opening a card shows the thread and the composer"
    );
    let channels = function_body(js, "loadChannels");
    assert!(
        channels.contains("selectedThreadId = null") && channels.contains("syncCollabPanel()"),
        "leaving a channel hides the composer again"
    );
}

/// A lane is space, not a box. No tinted column, no colored dot, no white
/// count pill. The heading is the lane name and its number, in 12px muted type.
#[test]
fn ui_js_lanes_are_space_not_boxes() {
    assert!(
        page().contains(".board-col { background: transparent; border: 0; box-shadow: none; padding: 0; min-height: 0; }"),
        "a lane has no background, border, padding, or shadow"
    );
    assert!(
        !page().contains("#efece5")
            && !page().contains(".board-col .dot")
            && !page().contains(".board-col h3 .count"),
        "the tinted box, the colored dot, and the white count pill are gone"
    );
    assert!(
        page().contains("gap: 16px")
            && page().contains(".board-col h3 { margin: 0 0 8px; font-size: 12px; font-weight: 400; letter-spacing: 0; text-transform: none; color: #63636c; }"),
        "lanes sit 16px apart and the heading is 12px muted type, not uppercase tracking"
    );
    let board = function_body(script(HTML), "renderBoard");
    assert!(
        !board.contains("className = \"dot\"") && !board.contains("className = \"count\""),
        "renderBoard does not paint a dot or a count pill"
    );
    assert!(
        board.contains("label.textContent = c.title")
            && board.contains("h.append(label, \" \", count)"),
        "the lane heading is the name plus the number"
    );
    assert!(
        board.contains("col.count.textContent = String(col.n)"),
        "the number is the count of cards in that lane"
    );
}

/// State on a card is a word in the foot, not a pill. The legend is gone.
/// The open thread uses the same word. "Review" and "Approval" are words,
/// not a tinted chip.
#[test]
fn ui_js_state_is_a_word_not_a_pill() {
    assert!(
        !page().contains("legend-box")
            && !page().contains("What the badges mean")
            && !page().contains("chrome-badge"),
        "the badge legend and the pill class are gone"
    );
    assert!(
        page().contains(".card-foot { display: flex; align-items: center; gap: 0.4rem; font-size: 12px; font-weight: 400; color: #63636c; }"),
        "the card foot is 12px muted type"
    );
    assert!(
        page().contains("#thread-badge { font-size: 12px; font-weight: 400; color: #63636c; }"),
        "the thread state is the same word, not a pill"
    );
    assert!(
        page().contains(
            ".ny-kind { font-size: 12px; font-weight: 400; color: #63636c; white-space: nowrap; }"
        ) && !page().contains(".ny-kind.gate")
            && !page().contains("#ffedd5"),
        "Review and Approval are words, not a tinted chip"
    );
    let js = script(HTML);
    let board = function_body(js, "renderBoard");
    assert!(
        !board.contains("chromeBadge(")
            && board.contains("stateWord.textContent = chrome.label")
            && board.contains("card.append(t, foot)"),
        "renderBoard puts the sessionChrome label in the card foot and does not paint a badge"
    );
    assert!(
        board.contains("card.setAttribute(\"aria-label\", `${title}: ${chrome.label}`)"),
        "the card's accessible name uses the same state word"
    );
    let header = function_body(js, "renderThreadHeader");
    assert!(
        !header.contains("chromeBadge(")
            && header.contains("badgeBox.textContent = sessionChrome(th, lastGates[tid]).label"),
        "the thread header shows the state word"
    );
    let row = function_body(js, "needsYouRow");
    assert!(
        row.contains("kind.textContent = item.kind === \"open_gate\" ? \"Approval\" : \"Review\""),
        "a needs-you row says Review or Approval"
    );
}

/// A Needs you refusal stays on the row the person is reading. A reload
/// used to rebuild that row and drop the error, so the browser spec timed
/// out waiting for a sentence that had already been painted on a detached
/// node.
#[test]
fn ui_js_needs_you_error_stays_on_the_row() {
    let js = script(HTML);
    let render = function_body(js, "renderNeedsYou");
    assert!(
        render.contains("querySelector(\".ny-err\")") && render.contains("keep.set"),
        "a reload keeps the row that is showing an error"
    );
    let approve = function_body(js, "approveFromInbox");
    assert!(
        approve.contains("rowOnScreen"),
        "a refusal is painted on the row that is on screen after the request"
    );
}

/// `#team` is only people holding work. From docs/UI Design.md:
/// do not append a chip whose state is `idle`; the viewer may still show
/// when `needsYou.length` is non-zero ("N waiting on you"); when someone
/// holds two open tasks, the title is the `running` one, else the one with
/// the latest `updated_at`. Not "the first non-closed task unless a later
/// one is running."
///
/// This runs the page's `renderTeam`. It fails if a person who is not
/// holding work is shown, or if a person who is holding work is hidden.
#[test]
fn ui_js_team_is_only_people_holding_work() {
    let scenes = team_scenes();
    let rendered = render_team_in_page(&scenes);
    for scene in &scenes {
        let actual = rendered
            .get(scene.name)
            .unwrap_or_else(|| panic!("no render for {}", scene.name));
        let expected = team_the_rule_requires(scene);
        for (id, text) in actual {
            assert!(
                expected.contains_key(id.as_str()),
                "{}: showed {id} ({text}), who is not holding work",
                scene.name
            );
        }
        for (id, want) in &expected {
            let got = actual.get(*id).unwrap_or_else(|| {
                panic!(
                    "{}: hid {id}, who is holding work{}",
                    scene.name,
                    if *id == scene.viewer {
                        " (or is the viewer, with work waiting)"
                    } else {
                        ""
                    }
                )
            });
            // A held task is "label · title". The rule picks the title.
            // The viewer's waiting line has no separator.
            let line = got.split_once(" · ").map(|(_, title)| title).unwrap_or(got);
            assert_eq!(line, want, "{}", scene.name);
        }
    }
}

struct Mate {
    id: &'static str,
    name: &'static str,
    kind: &'static str,
    live: bool,
}

struct OpenTask {
    id: &'static str,
    assignee_id: Option<&'static str>,
    state: &'static str,
    title: &'static str,
    updated_at: &'static str,
    work_started_at: Option<&'static str>,
    /// Pending gate. `Some(true)` means the gate has a schema.
    gate_schema: Option<bool>,
}

struct TeamScene {
    name: &'static str,
    viewer: &'static str,
    needs_you: usize,
    people: Vec<Mate>,
    threads: Vec<OpenTask>,
}

fn mate(id: &'static str, name: &'static str, kind: &'static str, live: bool) -> Mate {
    Mate {
        id,
        name,
        kind,
        live,
    }
}

fn task(
    id: &'static str,
    assignee: Option<&'static str>,
    state: &'static str,
    title: &'static str,
    updated_at: &'static str,
) -> OpenTask {
    OpenTask {
        id,
        assignee_id: assignee,
        state,
        title,
        updated_at,
        work_started_at: None,
        gate_schema: None,
    }
}

fn running(
    id: &'static str,
    assignee: &'static str,
    title: &'static str,
    updated_at: &'static str,
) -> OpenTask {
    OpenTask {
        work_started_at: Some(updated_at),
        ..task(id, Some(assignee), "working", title, updated_at)
    }
}

fn team_scenes() -> Vec<TeamScene> {
    vec![
        TeamScene {
            name: "idle people stay off, including a live one; holders stay on, including an offline human",
            viewer: "you",
            needs_you: 0,
            people: vec![
                mate("you", "You", "human", true),
                mate("ada", "Ada", "agent", true),
                mate("ben", "Ben", "agent", false),
                mate("cara", "Cara", "human", false),
                mate("dan", "Dan", "human", true),
                mate("erin", "Erin", "human", false),
            ],
            threads: vec![
                task("t-ben", Some("ben"), "working", "Hold the line", "2026-10-01T00:00:00Z"),
                task("t-cara", Some("cara"), "working", "File the note", "2026-10-01T00:00:00Z"),
                task("t-erin", Some("erin"), "closed", "Already done", "2026-10-03T00:00:00Z"),
            ],
        },
        TeamScene {
            name: "two claimed tasks title the latest updated_at, not the first",
            viewer: "you",
            needs_you: 0,
            people: vec![mate("you", "You", "human", false), mate("ben", "Ben", "agent", false)],
            threads: vec![
                task("a", Some("ben"), "working", "Alpha", "2026-10-01T00:00:00Z"),
                task("b", Some("ben"), "working", "Beta", "2026-10-03T00:00:00Z"),
            ],
        },
        TeamScene {
            name: "two running tasks title the latest updated_at, not the later running one",
            viewer: "you",
            needs_you: 0,
            people: vec![mate("you", "You", "human", false), mate("ben", "Ben", "agent", false)],
            threads: vec![
                running("g", "ben", "Gamma", "2026-10-04T00:00:00Z"),
                running("d", "ben", "Delta", "2026-10-02T00:00:00Z"),
            ],
        },
        TeamScene {
            name: "the running task wins over a newer claimed one",
            viewer: "you",
            needs_you: 0,
            people: vec![mate("you", "You", "human", false), mate("ben", "Ben", "agent", false)],
            threads: vec![
                task("e", Some("ben"), "working", "Epsilon", "2026-10-05T00:00:00Z"),
                running("z", "ben", "Zeta", "2026-10-01T00:00:00Z"),
            ],
        },
        TeamScene {
            name: "the running task wins over a newer in-review one",
            viewer: "you",
            needs_you: 0,
            people: vec![mate("you", "You", "human", false), mate("ben", "Ben", "agent", false)],
            threads: vec![
                running("eta", "ben", "Eta", "2026-10-01T00:00:00Z"),
                task("theta", Some("ben"), "in_review", "Theta", "2026-10-06T00:00:00Z"),
            ],
        },
        TeamScene {
            name: "the running task wins over a newer gated one",
            viewer: "you",
            needs_you: 0,
            people: vec![mate("you", "You", "human", false), mate("ben", "Ben", "agent", false)],
            threads: vec![
                running("iota", "ben", "Iota", "2026-10-01T00:00:00Z"),
                OpenTask {
                    gate_schema: Some(false),
                    ..task("kappa", Some("ben"), "working", "Kappa", "2026-10-07T00:00:00Z")
                },
            ],
        },
        TeamScene {
            name: "closed and archived tasks are not work being held",
            viewer: "you",
            needs_you: 0,
            people: vec![
                mate("you", "You", "human", false),
                mate("ben", "Ben", "agent", true),
                mate("cara", "Cara", "human", false),
            ],
            threads: vec![
                OpenTask {
                    work_started_at: Some("2026-10-08T00:00:00Z"),
                    ..task("gone", Some("ben"), "archived", "Gone", "2026-10-08T00:00:00Z")
                },
                task("old", Some("cara"), "closed", "Old", "2026-10-01T00:00:00Z"),
                task("open", Some("cara"), "working", "Still open", "2026-10-02T00:00:00Z"),
            ],
        },
        TeamScene {
            name: "the viewer shows while work is waiting, and says how many",
            viewer: "you",
            needs_you: 3,
            people: vec![mate("you", "You", "human", false), mate("ben", "Ben", "agent", false)],
            threads: vec![
                task("old", Some("you"), "closed", "Old", "2026-10-01T00:00:00Z"),
                task("b", Some("ben"), "working", "Hold the line", "2026-10-02T00:00:00Z"),
            ],
        },
        TeamScene {
            name: "a viewer who holds work shows that task, not the waiting line",
            viewer: "you",
            needs_you: 3,
            people: vec![mate("you", "You", "human", false)],
            threads: vec![task("yours", Some("you"), "working", "Your task", "2026-10-02T00:00:00Z")],
        },
        TeamScene {
            name: "an unassigned task puts nobody on the strip",
            viewer: "you",
            needs_you: 0,
            people: vec![mate("you", "You", "human", true), mate("ada", "Ada", "agent", true)],
            threads: vec![task("free", None, "open", "Free", "2026-10-01T00:00:00Z")],
        },
    ]
}

/// Who the strip shows, and the line under their name.
///
/// Holding an open task means assigned, and not `closed` or `archived`.
/// `running` is that task's session chrome: work has started, and it is not
/// in review and not waiting on a gate. The title is the running task when
/// there is one, otherwise the latest `updated_at`.
fn team_the_rule_requires(scene: &TeamScene) -> std::collections::BTreeMap<&'static str, String> {
    let mut held: std::collections::BTreeMap<&str, Vec<&OpenTask>> =
        std::collections::BTreeMap::new();
    for th in &scene.threads {
        if th.assignee_id.is_some() && th.state != "closed" && th.state != "archived" {
            held.entry(th.assignee_id.unwrap()).or_default().push(th);
        }
    }
    let mut out = std::collections::BTreeMap::new();
    for person in &scene.people {
        if person.kind != "agent" && person.kind != "human" {
            continue;
        }
        if let Some(tasks) = held.get(person.id) {
            let chosen = title_task(tasks);
            out.insert(person.id, chosen.title.to_string());
        } else if person.id == scene.viewer && scene.needs_you > 0 {
            out.insert(person.id, format!("{} waiting on you", scene.needs_you));
        }
    }
    out
}

fn title_task<'a>(tasks: &[&'a OpenTask]) -> &'a OpenTask {
    let running: Vec<_> = tasks.iter().copied().filter(|th| is_running(th)).collect();
    let pool: Vec<&OpenTask> = if running.is_empty() {
        tasks.to_vec()
    } else {
        running
    };
    pool.into_iter()
        .max_by_key(|th| th.updated_at)
        .expect("a held task")
}

fn is_running(th: &OpenTask) -> bool {
    th.assignee_id.is_some()
        && th.work_started_at.is_some()
        && th.gate_schema.is_none()
        && th.state != "closed"
        && th.state != "archived"
        && th.state != "in_review"
}

/// Execute the page's `sessionChrome` and `renderTeam` for each scene.
/// The oracle above is the rule; this only reports what the page drew.
fn render_team_in_page(
    scenes: &[TeamScene],
) -> std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>> {
    let payload = serde_json::json!({
        "html": HTML,
        "js": script(HTML),
        "css": UI_CSS,
        "scenes": scenes.iter().map(scene_payload).collect::<Vec<_>>(),
    });
    let mut child = std::process::Command::new("node")
        .arg("-e")
        .arg(TEAM_PAGE_HARNESS)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap_or_else(|err| panic!("node is required to run renderTeam: {err}"));
    serde_json::to_writer(child.stdin.take().expect("stdin"), &payload).expect("write scenes");
    let out = child.wait_with_output().expect("node");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "renderTeam harness failed\n{stderr}\n{stdout}"
    );
    let value: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|err| panic!("renderTeam harness returned {err}: {stdout}"));
    let mut rendered = std::collections::BTreeMap::new();
    for scene in value.as_array().expect("scene list") {
        let name = scene["name"].as_str().expect("name").to_string();
        let mut chips = std::collections::BTreeMap::new();
        for chip in scene["chips"].as_array().expect("chips") {
            let id = chip["id"].as_str().expect("id").to_string();
            let text = chip["text"].as_str().expect("text").to_string();
            assert!(
                chips.insert(id.clone(), text.clone()).is_none(),
                "{name}: {id} was drawn twice"
            );
        }
        rendered.insert(name, chips);
    }
    rendered
}

fn scene_payload(scene: &TeamScene) -> serde_json::Value {
    serde_json::json!({
        "name": scene.name,
        "viewer": scene.viewer,
        "needsYou": scene.needs_you,
        "people": scene.people.iter().map(|p| serde_json::json!({
            "id": p.id,
            "name": p.name,
            "kind": p.kind,
            "live": p.live,
        })).collect::<Vec<_>>(),
        "threads": scene.threads.iter().map(|th| {
            let mut v = serde_json::json!({
                "id": th.id,
                "state": th.state,
                "title": th.title,
                "updated_at": th.updated_at,
            });
            if let Some(id) = th.assignee_id {
                v["assignee_id"] = serde_json::json!(id);
            }
            if let Some(at) = th.work_started_at {
                v["work_started_at"] = serde_json::json!(at);
            }
            if let Some(schema) = th.gate_schema {
                v["gate"] = serde_json::json!({ "hasSchema": schema });
            }
            v
        }).collect::<Vec<_>>(),
    })
}

const TEAM_PAGE_HARNESS: &str = r#"
const fs = require("fs");
const input = JSON.parse(fs.readFileSync(0, "utf8"));
const html = input.html;
const js = input.js;
function functionSource(name) {
  const key = "function " + name + "(";
  const at = js.indexOf(key);
  if (at < 0) throw new Error("missing " + name);
  const rel = js.slice(at).indexOf("\n      }\n");
  if (rel < 0) throw new Error("unclosed " + name);
  return js.slice(at, at + rel) + "\n      }";
}
const sessionChrome = new Function(functionSource("sessionChrome") + "\nreturn sessionChrome;")();
const renderTeam = new Function(
  "document",
  "memberDirectory",
  "lastGates",
  "needsYou",
  "authorId",
  "lastSeen",
  "LIVE_MS",
  "avatarEl",
  "sessionChrome",
  functionSource("renderTeam") + "\nreturn renderTeam;"
);
function makeEl(tag) {
  return {
    tag,
    children: [],
    dataset: {},
    className: "",
    textContent: "",
    title: "",
    appendChild(child) { this.children.push(child); return child; },
    append(...kids) { for (const kid of kids) this.children.push(kid); },
    replaceChildren() { this.children = []; },
  };
}
const results = input.scenes.map((scene) => {
  const teamBox = makeEl("div");
  const document = {
    getElementById(id) {
      if (id !== "team") throw new Error("unexpected element " + id);
      return teamBox;
    },
    createElement(tag) { return makeEl(tag); },
  };
  const memberDirectory = new Map(scene.people.map((p) => [p.id, { name: p.name, kind: p.kind }]));
  const lastGates = {};
  const threads = scene.threads.map((th) => {
    const copy = Object.assign({}, th);
    if (copy.gate) lastGates[copy.id] = copy.gate;
    delete copy.gate;
    return copy;
  });
  const needsYou = Array.from({ length: scene.needsYou }, () => ({}));
  const now = Date.now();
  const lastSeen = new Map(scene.people.filter((p) => p.live).map((p) => [p.id, now]));
  const avatarEl = () => makeEl("span");
  renderTeam(
    document,
    memberDirectory,
    lastGates,
    needsYou,
    () => scene.viewer,
    lastSeen,
    120000,
    avatarEl,
    sessionChrome
  )(threads);
  const chips = teamBox.children.map((chip) => {
    const text = chip.children.find((child) => child.className === "mate-text");
    const small = text.children.find((child) => child.tag === "small");
    return { id: chip.dataset.memberId, text: small.textContent };
  });
  return { name: scene.name, chips };
});
process.stdout.write(JSON.stringify(results));
"#;

/// QA items from #1141 that were still true on main: a dead server must not
/// surface as `TypeError: Failed to fetch` or as a lost post, a 403 search
/// must not dump problem JSON, Open DM selects the conversation and names the
/// other member, Add task does not claim to need a bearer, and a token that
/// is accepted clears the rejection that came before it.
#[test]
fn ui_js_reports_qa_failures_in_words_and_opens_the_dm() {
    let js = script(HTML);
    let channels = function_body(js, "loadChannels");
    assert!(
        channels.contains("renderState(list, unreachable(e), \"err\")"),
        "a channel refresh that cannot reach the server uses the friendly copy"
    );
    assert!(
        !channels.contains("String(e)"),
        "a channel refresh must not render the raw exception"
    );

    let post = js
        .split("getElementById(\"post-message\").onclick")
        .nth(1)
        .expect("post handler");
    let post = &post[..post
        .find("getElementById(\"reload-messages\")")
        .expect("next handler")];
    assert!(
        post.contains("catch (e)") && post.contains("showError(unreachable(e))"),
        "a post that never reaches the server is reported"
    );
    let refused = post.find("if (!res.ok)").expect("http failure branch");
    let cleared = post
        .find("compose-body\").value = \"\"")
        .expect("composer clear");
    assert!(
        refused < cleared,
        "the draft is cleared only after the server accepts the post"
    );

    let dms = function_body(js, "loadDms");
    assert!(
        dms.contains("memberName(other)") && dms.contains("await loadMembers()"),
        "a DM list names the other member"
    );
    assert!(
        !dms.contains("with ${dmOther(c)}"),
        "a DM list must not show the raw member id"
    );
    let open = function_body(js, "openDm");
    assert!(
        open.contains("selectDm(opened)"),
        "opening a DM selects that conversation"
    );

    assert!(
        !page().contains("title=\"Requires bearer token\""),
        "Add task and new channel no longer say they require a bearer"
    );
    assert!(
        page().contains("title=\"Sign in or paste a token\""),
        "the write buttons say a session or a token is enough"
    );

    let search = js
        .split("getElementById(\"run-search\").onclick")
        .nth(1)
        .expect("search handler");
    let search = &search[..search
        .find("getElementById(\"create-channel\")")
        .expect("next")];
    assert!(
        search.contains("humanError(res.status, detail)")
            && !search.contains("box.textContent = body"),
        "a refused search is words, not the raw problem body"
    );

    let token = js
        .split("getElementById(\"token\").addEventListener(\"change\"")
        .nth(1)
        .expect("token change");
    assert!(
        token.contains("status.hidden = true;\n        status.textContent = \"\";"),
        "an accepted token clears the earlier rejection"
    );
}

#[test]
fn ui_js_a_session_edits_uploads_and_decides_without_a_bearer() {
    let s = script(HTML);
    assert!(
        s.contains("function apiReadPath(") && s.contains("function apiWritePath("),
        "reads and writes share a pair of path helpers"
    );
    assert!(
        !s.contains("function requireTokenForWrite("),
        "the bearer-only write guard is replaced, not kept beside the session path"
    );
    assert!(
        s.contains("apiWritePath(`/messages/${id}`)")
            && s.contains("apiWritePath(`/artifacts?${params}`)")
            && s.contains("apiWritePath(`/threads/${tid}/reviews`)")
            && s.contains("apiWritePath(`/threads/${tid}`)"),
        "edit, upload, review and close go through the session proxy"
    );
    assert!(
        s.contains("apiReadPath(`/threads/${tid}/review-status`)")
            && s.contains("apiReadPath(`/threads/${id}`)")
            && s.contains("apiReadPath(`/workspaces/${wid()}`)"),
        "thread, review status and workspace reads go through the session proxy"
    );
    assert!(
        s.contains("This needs a bearer token. A signed-in session cannot call it."),
        "a bearer-only action tells a session so, instead of sending it to a raw error"
    );
    assert!(
        !s.contains("a browser session cannot transition"),
        "a session can start a review and close a task"
    );
}

/// One channel hides the sidebar. From docs/UI Design.md:
/// in `loadChannels`, if `channels.length === 1`, hide `aside`.
/// At two or more, show `aside`. The channel name is already `#board-title`.
///
/// This runs the page's `loadChannels`. It fails if the sidebar stays visible
/// when there is only one channel, and fails if it hides when there is more
/// than one. The expected visibility is that rule, not a copy of the markup.
#[test]
fn ui_js_one_channel_hides_the_sidebar() {
    assert_eq!(
        HTML.matches("<aside").count(),
        1,
        "one sidebar, not a second one"
    );
    let counts = [1usize, 2, 8, 1, 0, 3];
    let rendered = sidebar_after_load_channels(&counts);
    for count in counts {
        let hidden = rendered[&count];
        let want = the_design_hides_the_sidebar(count);
        if want {
            assert!(
                hidden,
                "{count} channel: sidebar stayed visible; one channel hides it"
            );
        } else if count > 1 {
            assert!(
                !hidden,
                "{count} channels: sidebar hid; two or more show it"
            );
        } else {
            assert!(
                !hidden,
                "no channels: sidebar hid; the rule hides it only when there is one"
            );
        }
    }
}

/// Hide the sidebar only when the workspace has exactly one channel.
fn the_design_hides_the_sidebar(channel_count: usize) -> bool {
    channel_count == 1
}

/// What `loadChannels` did to `aside` for each channel count.
/// A missing sidebar, `hidden`, or `display: none` counts as hidden.
fn sidebar_after_load_channels(counts: &[usize]) -> std::collections::BTreeMap<usize, bool> {
    let payload = serde_json::json!({ "html": HTML,
        "js": script(HTML),
        "css": UI_CSS, "counts": counts });
    let mut child = std::process::Command::new("node")
        .arg("-e")
        .arg(SIDEBAR_PAGE_HARNESS)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap_or_else(|err| panic!("node is required to run loadChannels: {err}"));
    serde_json::to_writer(child.stdin.take().expect("stdin"), &payload).expect("write counts");
    let out = child.wait_with_output().expect("node");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "loadChannels harness failed\n{stderr}\n{stdout}"
    );
    let value: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|err| panic!("loadChannels harness returned {err}: {stdout}"));
    let mut rendered = std::collections::BTreeMap::new();
    for scene in value.as_array().expect("count list") {
        let count = scene["count"].as_u64().expect("count") as usize;
        let hidden = scene["hidden"].as_bool().expect("hidden");
        rendered.insert(count, hidden);
    }
    assert_eq!(
        rendered.len(),
        counts
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        "each channel count was loaded"
    );
    rendered
}

const SIDEBAR_PAGE_HARNESS: &str = r##"
const fs = require("fs");
const input = JSON.parse(fs.readFileSync(0, "utf8"));
const html = input.html;
const js = input.js;
function functionSource(name) {
  const key = "async function " + name + "(";
  const at = js.indexOf(key);
  if (at < 0) throw new Error("missing " + name);
  const rel = js.slice(at).indexOf("\n      }\n");
  if (rel < 0) throw new Error("unclosed " + name);
  return js.slice(at, at + rel) + "\n      }";
}
function makeEl(tag) {
  const el = {
    tag,
    hidden: false,
    children: [],
    dataset: {},
    className: "",
    textContent: "",
    value: "",
    title: "",
    attrs: {},
    style: { display: "" },
    classList: { add() {}, remove() {}, toggle() {} },
    appendChild(child) { this.children.push(child); return child; },
    append(...kids) { for (const kid of kids) this.children.push(kid); },
    replaceChildren() { this.children = []; },
    setAttribute(k, v) {
      this.attrs[k] = String(v);
      if (k === "hidden") this.hidden = true;
    },
    getAttribute(k) {
      return Object.prototype.hasOwnProperty.call(this.attrs, k) ? this.attrs[k] : null;
    },
    removeAttribute(k) {
      delete this.attrs[k];
      if (k === "hidden") this.hidden = false;
    },
    querySelector(sel) {
      const all = this.querySelectorAll(sel);
      return all[0] || null;
    },
    querySelectorAll(sel) {
      if (sel === "li[data-id]" || sel.indexOf("li[data-id=") === 0) {
        const mark = 'li[data-id="';
        let id = null;
        if (sel.indexOf(mark) === 0 && sel.endsWith('"]')) id = sel.slice(mark.length, -2);
        return this.children.filter((c) => c.tag === "li" && c.dataset.id && (id === null || c.dataset.id === id));
      }
      return [];
    },
    click() { if (typeof this.onclick === "function") this.onclick(); },
  };
  Object.defineProperty(el, "innerHTML", {
    set() { el.children = []; },
    get() { return ""; },
  });
  return el;
}
const list = makeEl("ul");
const aside = makeEl("aside");
const byId = {};
function element(id) {
  if (id === "channel-list") return list;
  if (!byId[id]) byId[id] = makeEl(id);
  return byId[id];
}
const document = {
  getElementById(id) { return element(id); },
  querySelector(sel) { return sel === "aside" ? aside : null; },
  querySelectorAll(sel) {
    if (sel === "#channel-list li") return list.children.filter((c) => c.tag === "li");
    return [];
  },
  createElement(tag) { return makeEl(tag); },
};
const store = {};
const localStorage = {
  getItem(k) { return Object.prototype.hasOwnProperty.call(store, k) ? store[k] : null; },
  setItem(k, v) { store[k] = String(v); },
};
let current = [];
async function fetch() {
  return { ok: true, json: async () => current };
}
const loadChannels = new Function(
  "document",
  "fetch",
  "localStorage",
  `
    const CSS = { escape(s) { return String(s); } };
    let selectedChannelId = null;
    let selectedChannelName = null;
    let selectedThreadId = null;
    let boardSeen = new Map();
    let headerGen = 0;
    let sessionMemberId = null;
    const channelKey = "maidan_channel";
    function wid() { return "ws"; }
    function renderState() {}
    function setLoading() {}
    function clearLoading() {}
    function persist() {}
    function token() { return ""; }
    function base() { return "http://maidan.test"; }
    function uiReadPath(s) { return s; }
    function headers() { return {}; }
    async function responseError() { return "no"; }
    async function loadMembers() {}
    function escapeHtml(s) { return String(s); }
    function keyActivates() {}
    function syncCollabPanel() {}
    function unreachable(e) { return String(e && e.message || e); }
    function loadThreads() {}
    function api(url, options) { return fetch(url, options); }
    ${functionSource("loadChannels")}
    return loadChannels;
  `
)(document, fetch, localStorage);
function isHidden(el) {
  if (!el) return true;
  if (el.hidden === true) return true;
  if (el.style && el.style.display === "none") return true;
  return false;
}
(async () => {
  const results = [];
  for (const count of input.counts) {
    current = Array.from({ length: count }, (_, i) => ({
      id: "c" + i,
      name: "Channel " + i,
      private: false,
    }));
    await loadChannels();
    results.push({ count, hidden: isHidden(aside) });
  }
  process.stdout.write(JSON.stringify(results));
})().catch((err) => {
  console.error(err);
  process.exit(1);
});
"##;

/// No refusal strip. From docs/UI Design.md:
/// do not paint `#board-refusal`. Keep `.card-refusal` with the text
/// "Close refused" on that `.card`. The server's sentence can be the card
/// `title`. Do not drop the refusal itself.
///
/// This runs the page's `rememberRefusal` and `renderBoard`. It fails if
/// `#board-refusal` is shown or painted, including when the refusal arrives
/// before the board has any cards (that used to open the strip). A refused
/// card keeps the line "Close refused" and the server's sentence as its
/// title. A card that was not refused does not grow either.
#[test]
fn ui_js_no_refusal_strip() {
    let file = "Close needs a review. Next: ask a reviewer.";
    let ship = "The result is still open. Next: post the result.";
    let older = "An earlier close was refused. Next: wait.";
    let steps = [
        RefusalStep::remember("t1", "m-1", file, "2026-10-01T12:00:00Z"),
        RefusalStep::render(&[
            RefusedTask::new("t1", "File the notes", "in_review"),
            RefusedTask::new("t2", "Ship the summary", "open"),
        ]),
        RefusalStep::remember("t2", "m-2", ship, "2026-10-01T12:05:00Z"),
        RefusalStep::remember("t1", "m-1", older, "2026-10-01T11:00:00Z"),
        RefusalStep::render(&[
            RefusedTask::new("t1", "File the notes", "in_review"),
            RefusedTask::new("t2", "Ship the summary", "open"),
        ]),
    ];
    let drawn = board_after_refusals(&steps);
    assert_eq!(drawn.len(), steps.len(), "every step was painted");

    // What the rule keeps. A later notice replaces an earlier one. An older
    // stamp does not. The banner is never part of this.
    let mut sentence = std::collections::BTreeMap::new();
    let mut stamp = std::collections::BTreeMap::new();
    let mut on_board = std::collections::BTreeMap::new();
    for (step, view) in steps.iter().zip(drawn.iter()) {
        match step {
            RefusalStep::Remember { id, text, at, .. } => {
                let replace = match stamp.get(id) {
                    Some(prev) => *prev <= *at,
                    None => true,
                };
                if replace {
                    sentence.insert(*id, *text);
                    stamp.insert(*id, *at);
                }
            }
            RefusalStep::Render { tasks } => {
                on_board.clear();
                for task in tasks.iter() {
                    on_board.insert(task.id, task.title);
                }
            }
        }
        assert!(
            !view.banner_shown && view.banner_text.is_empty() && !view.refusal_text_in_banner,
            "painted a refusal banner at {}: shown {} text {:?} refusal-text {}",
            step.label(),
            view.banner_shown,
            view.banner_text,
            view.refusal_text_in_banner
        );
        assert_eq!(
            view.cards.len(),
            on_board.len(),
            "{}: card count",
            step.label()
        );
        for card in &view.cards {
            let task_title = on_board.get(card.id.as_str()).unwrap_or_else(|| {
                panic!(
                    "{}: showed {}, which is not on the board",
                    step.label(),
                    card.id
                )
            });
            assert_eq!(
                card.heading.as_str(),
                *task_title,
                "{}: {}",
                step.label(),
                card.id
            );
            match sentence.get(card.id.as_str()) {
                Some(words) => {
                    assert_eq!(
                        card.line.as_deref(),
                        Some("Close refused"),
                        "{}: {} dropped the refusal",
                        step.label(),
                        card.id
                    );
                    assert!(
                        card.title == *words || card.heading == *words,
                        "{}: {} sentence {words:?} is not the card title ({:?} / {:?})",
                        step.label(),
                        card.id,
                        card.title,
                        card.heading
                    );
                }
                None => {
                    assert!(
                        card.line.is_none(),
                        "{}: {} showed a refusal it does not have ({:?})",
                        step.label(),
                        card.id,
                        card.line
                    );
                    assert!(
                        card.title.is_empty(),
                        "{}: {} titled a card that was not refused",
                        step.label(),
                        card.id
                    );
                }
            }
        }
        for id in sentence.keys() {
            if on_board.contains_key(id) {
                assert!(
                    view.cards.iter().any(|card| card.id == *id),
                    "{}: hid {id}, whose close was refused",
                    step.label()
                );
            }
        }
    }
}

#[derive(Clone)]
struct RefusedTask {
    id: &'static str,
    title: &'static str,
    state: &'static str,
}

impl RefusedTask {
    fn new(id: &'static str, title: &'static str, state: &'static str) -> Self {
        Self { id, title, state }
    }
}

enum RefusalStep {
    Remember {
        id: &'static str,
        actor: &'static str,
        text: &'static str,
        at: &'static str,
    },
    Render {
        tasks: Vec<RefusedTask>,
    },
}

impl RefusalStep {
    fn remember(
        id: &'static str,
        actor: &'static str,
        text: &'static str,
        at: &'static str,
    ) -> Self {
        Self::Remember {
            id,
            actor,
            text,
            at,
        }
    }

    fn render(tasks: &[RefusedTask]) -> Self {
        Self::Render {
            tasks: tasks.to_vec(),
        }
    }

    fn label(&self) -> String {
        match self {
            Self::Remember { id, at, .. } => format!("remember {id} @ {at}"),
            Self::Render { tasks } => format!("render {}", tasks.len()),
        }
    }
}

struct RefusalCard {
    id: String,
    heading: String,
    line: Option<String>,
    title: String,
}

struct RefusalView {
    banner_shown: bool,
    banner_text: String,
    refusal_text_in_banner: bool,
    cards: Vec<RefusalCard>,
}

fn board_after_refusals(steps: &[RefusalStep]) -> Vec<RefusalView> {
    let payload = serde_json::json!({
        "html": HTML,
        "js": script(HTML),
        "css": UI_CSS,
        "steps": steps.iter().map(refusal_step_payload).collect::<Vec<_>>(),
    });
    let mut child = std::process::Command::new("node")
        .arg("-e")
        .arg(REFUSAL_PAGE_HARNESS)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap_or_else(|err| panic!("node is required to run the board: {err}"));
    serde_json::to_writer(child.stdin.take().expect("stdin"), &payload).expect("write steps");
    let out = child.wait_with_output().expect("node");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "refusal harness failed\n{stderr}\n{stdout}"
    );
    let value: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|err| panic!("refusal harness returned {err}: {stdout}"));
    value
        .as_array()
        .expect("step list")
        .iter()
        .map(|scene| RefusalView {
            banner_shown: scene["banner_shown"].as_bool().expect("banner_shown"),
            banner_text: scene["banner_text"]
                .as_str()
                .expect("banner_text")
                .to_string(),
            refusal_text_in_banner: scene["refusal_text_in_banner"]
                .as_bool()
                .expect("refusal_text_in_banner"),
            cards: scene["cards"]
                .as_array()
                .expect("cards")
                .iter()
                .map(|card| RefusalCard {
                    id: card["id"].as_str().expect("id").to_string(),
                    heading: card["heading"].as_str().expect("heading").to_string(),
                    line: card["line"].as_str().map(str::to_string),
                    title: card["title"].as_str().expect("title").to_string(),
                })
                .collect(),
        })
        .collect()
}

fn refusal_step_payload(step: &RefusalStep) -> serde_json::Value {
    match step {
        RefusalStep::Remember {
            id,
            actor,
            text,
            at,
        } => serde_json::json!({
            "op": "remember",
            "id": id,
            "actor": actor,
            "text": text,
            "at": at,
        }),
        RefusalStep::Render { tasks } => serde_json::json!({
            "op": "render",
            "threads": tasks.iter().map(|task| serde_json::json!({
                "id": task.id,
                "title": task.title,
                "state": task.state,
                "updated_at": "2026-10-01T12:00:00Z",
            })).collect::<Vec<_>>(),
        }),
    }
}

const REFUSAL_PAGE_HARNESS: &str = r#####"
const fs = require("fs");
const input = JSON.parse(fs.readFileSync(0, "utf8"));
const html = input.html;
const js = input.js;
function functionSource(name) {
  const marks = ["\n      function " + name + "(", "\n      async function " + name + "("];
  let at = -1;
  for (const mark of marks) {
    const i = js.indexOf(mark);
    if (i >= 0) { at = i + 1; break; }
  }
  if (at < 0) throw new Error("missing " + name);
  const open = js.indexOf("{", at);
  let depth = 0;
  for (let j = open; j < js.length; j++) {
    const c = js[j];
    if (c === "{") depth++;
    else if (c === "}") {
      depth--;
      if (depth === 0) return js.slice(at, j + 1);
    }
  }
  throw new Error("unclosed " + name);
}
function extractConst(name) {
  const key = "const " + name + " = ";
  const at = js.indexOf(key);
  if (at < 0) throw new Error("missing " + name);
  const line = js.lastIndexOf("\n", at) + 1;
  let depth = 0;
  let started = false;
  for (let i = at; i < js.length; i++) {
    const c = js[i];
    if (c === "[" || c === "{") { depth++; started = true; }
    else if (c === "]" || c === "}") depth--;
    else if (c === ";" && started && depth === 0) return js.slice(line, i + 1);
  }
  throw new Error("unclosed " + name);
}
const prelude = `
function makeEl(tag) {
  const el = {
    tag: tag,
    id: "",
    hidden: false,
    children: [],
    dataset: {},
    className: "",
    textContent: "",
    title: "",
    style: {},
    attrs: {},
    tabIndex: 0,
  };
  el.classList = {
    add(name) { el.className = (el.className + " " + name).trim(); },
    remove() {},
    toggle() {},
  };
  el.appendChild = (child) => { el.children.push(child); return child; };
  el.append = (...kids) => {
    for (const kid of kids) {
      if (typeof kid === "string") {
        const text = makeEl("#text");
        text.textContent = kid;
        el.children.push(text);
      } else {
        el.children.push(kid);
      }
    }
  };
  el.replaceChildren = () => { el.children = []; };
  el.setAttribute = (key, value) => {
    el.attrs[key] = String(value);
    if (key === "hidden") el.hidden = true;
  };
  el.getAttribute = (key) => (Object.prototype.hasOwnProperty.call(el.attrs, key) ? el.attrs[key] : null);
  el.removeAttribute = (key) => { delete el.attrs[key]; };
  el.querySelector = (sel) => querySel(el, sel);
  return el;
}
function walk(el, fn) {
  for (const child of el.children || []) {
    fn(child);
    walk(child, fn);
  }
}
function classHas(el, name) {
  return (" " + String(el.className || "") + " ").indexOf(" " + name + " ") >= 0;
}
function querySel(root, sel) {
  if (sel === ".card-refusal") {
    let hit = null;
    walk(root, (el) => { if (!hit && classHas(el, "card-refusal")) hit = el; });
    return hit;
  }
  const mark = '#board .card[data-id="';
  if (sel.indexOf(mark) === 0 && sel.endsWith('"]')) {
    const id = sel.slice(mark.length, -2);
    let hit = null;
    walk(byId.board, (el) => {
      if (!hit && classHas(el, "card") && el.dataset && el.dataset.id === id) hit = el;
    });
    return hit;
  }
  return null;
}
function flatText(el) {
  if (!el) return "";
  if (!el.children || el.children.length === 0) return el.textContent || "";
  return el.children.map(flatText).join("");
}
const byId = {};
function element(id) {
  if (!byId[id]) throw new Error("unexpected element " + id);
  return byId[id];
}
for (const id of ["board", "board-summary", "board-title", "board-refusal", "team"]) {
  byId[id] = makeEl("div");
  byId[id].id = id;
}
byId["board-refusal"].hidden = true;
const document = {
  getElementById(id) { return element(id); },
  createElement(tag) { return makeEl(tag); },
  querySelector(sel) { return querySel(byId.board, sel); },
  querySelectorAll() { return []; },
};
const CSS = { escape(s) { return String(s); } };
let threadsById = new Map();
let lastGates = null;
let selectedChannelName = "Desk";
let selectedThreadId = null;
let boardSeen = new Map();
let selectedChannelId = "ch-1";
const refusals = new Map();
function personEl(id) {
  const el = document.createElement("span");
  el.className = "person";
  el.textContent = String(id);
  return el;
}
function ago() { return ""; }
function leaseLeft() { return ""; }
function boardRects() { return new Map(); }
function glideCards() {}
function renderTeam() {}
function emptyChannelHelp() { return document.createElement("div"); }
function selectThread() {}
`;
const tail = `
function snapshot() {
  const banner = byId["board-refusal"];
  let refusalText = false;
  walk(banner, (el) => { if (classHas(el, "refusal-text")) refusalText = true; });
  const cards = [];
  walk(byId.board, (el) => {
    if (!classHas(el, "card") || !el.dataset || !el.dataset.id) return;
    const headingEl = (el.children || []).find((child) => classHas(child, "card-title"));
    const lineEl = (el.children || []).find((child) => classHas(child, "card-refusal"));
    cards.push({
      id: el.dataset.id,
      heading: headingEl ? headingEl.textContent : "",
      line: lineEl ? lineEl.textContent : null,
      title: el.title || "",
    });
  });
  return {
    banner_shown: banner.hidden !== true,
    banner_text: flatText(banner),
    refusal_text_in_banner: refusalText,
    cards: cards,
  };
}
const out = [];
for (const step of steps) {
  if (step.op === "remember") rememberRefusal(step.id, step.actor, step.text, step.at);
  else if (step.op === "render") renderBoard(step.threads, {});
  else throw new Error("bad op " + step.op);
  out.push(snapshot());
}
return out;
`;
const body = prelude
  + functionSource("sessionChrome") + "\n"
  + extractConst("BOARD_COLUMNS") + "\n"
  + functionSource("paintRefusal") + "\n"
  + functionSource("rememberRefusal") + "\n"
  + functionSource("renderBoard") + "\n"
  + tail;
let views;
try {
  views = new Function("steps", body)(input.steps);
} catch (err) {
  console.error(err && err.stack || err);
  process.exit(1);
}
process.stdout.write(JSON.stringify(views));
"#####;

/// One sentence, one action, on an empty board. From docs/UI Design.md:
/// `emptyChannelHelp()` replaces the lanes with `#board-onboard`. The
/// primary action is **Connect an agent**. One sentence on how a task
/// arrives. No `POST /channels/…` path. No "pick a channel." No "New
/// thread in the sidebar." While `#first-run` is visible it is the only
/// primary, so that Connect button is a ghost. `#create-thread` is
/// `button.ghost`.
///
/// This runs the page's `emptyChannelHelp` and `renderBoard`, and reads
/// the static `#board`. It fails if the empty board is not exactly one
/// sentence and one action, if the lanes are still drawn, or if the
/// action is filled while first-run is on screen.
#[test]
fn ui_js_empty_board_is_one_sentence_and_one_action() {
    assert!(
        page().contains("<button id=\"create-thread\" type=\"button\" class=\"ghost\""),
        "#create-thread stays a ghost in #new-task"
    );
    let drawn = empty_boards_in_page();
    assert!(
        drawn.iter().any(|view| view.id == "static"),
        "the static empty board was read"
    );
    assert!(
        drawn
            .iter()
            .any(|view| view.id == "help" && !view.first_run_hidden),
        "emptyChannelHelp ran while first-run was visible"
    );
    assert!(
        drawn
            .iter()
            .any(|view| view.id == "help" && view.first_run_hidden),
        "emptyChannelHelp ran for an empty channel"
    );
    assert!(
        drawn
            .iter()
            .any(|view| view.id == "board" && view.first_run_hidden),
        "renderBoard painted an empty channel"
    );
    for view in &drawn {
        let prose = view
            .blocks
            .iter()
            .filter(|block| block.tag == "p")
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>();
        let actions = view
            .blocks
            .iter()
            .filter(|block| block.tag == "button")
            .collect::<Vec<_>>();
        let other = view
            .blocks
            .iter()
            .filter(|block| block.tag != "p" && block.tag != "button")
            .map(|block| format!("{} {:?}", block.tag, block.text))
            .collect::<Vec<_>>();
        assert!(
            other.is_empty(),
            "{}: the empty board has more than one sentence and one action ({})",
            view.label(),
            other.join(", ")
        );
        assert_eq!(
            prose.len(),
            1,
            "{}: expected one sentence, found {}",
            view.label(),
            prose.len()
        );
        assert_eq!(
            actions.len(),
            1,
            "{}: expected one action, found {}",
            view.label(),
            actions.len()
        );
        assert!(
            is_one_sentence(prose[0]),
            "{}: {:?} is not one sentence",
            view.label(),
            prose[0]
        );
        assert!(
            prose[0].to_lowercase().contains("task"),
            "{}: the sentence should say how a task arrives ({:?})",
            view.label(),
            prose[0]
        );
        let action = &actions[0].text;
        assert_eq!(
            action.as_str(),
            "Connect an agent",
            "{}: the action",
            view.label()
        );
        let class = actions[0].class_name.split_whitespace().collect::<Vec<_>>();
        if view.first_run_hidden {
            assert!(
                class.contains(&"primary") && !class.contains(&"ghost"),
                "{}: an empty channel's Connect an agent is the filled action, got {:?}",
                view.label(),
                actions[0].class_name
            );
        } else {
            assert!(
                class.contains(&"ghost") && !class.contains(&"primary"),
                "{}: while first-run is visible Connect an agent is a ghost, got {:?}",
                view.label(),
                actions[0].class_name
            );
        }
        let visible = view
            .blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        for banned in [
            "pick a channel",
            "New thread in the sidebar",
            "POST /channels",
        ] {
            assert!(
                !visible.contains(banned),
                "{}: still says {banned}",
                view.label()
            );
        }
        assert_eq!(
            view.lanes,
            0,
            "{}: an empty board draws no lanes",
            view.label()
        );
        assert_eq!(
            view.empty_lanes,
            0,
            "{}: an empty board draws no empty lane boxes",
            view.label()
        );
    }
}

/// A person never sees the raw error. From docs/UI Design.md: on `#toasts`,
/// `.ny-err`, `.onboard.board-error`, `#message-list`, and `#board-summary
/// .board-note`, the page shows the `humanError` sentence only. It does not
/// append the server body or ` (HTTP ${status})`. `pre#out` and `#live-feed`
/// may keep JSON; they sit behind More tools and `#live-more`.
///
/// This runs those surfaces against a problem document, a raw 500 body, a
/// capability refusal, and a fetch that throws. It fails if any of them shows
/// JSON, a stack, `HTTP`, or a status code.
#[test]
fn ui_js_never_shows_a_raw_error_or_an_http_code() {
    let report = person_facing_errors();
    let cases = report["cases"].as_array().expect("cases");
    assert!(!cases.is_empty(), "the page painted no error surfaces");
    let mut saw_refused = false;
    let mut saw_capability = false;
    let mut saw_server = false;
    let mut saw_not_found = false;
    let mut saw_unreachable = false;
    for case in cases {
        let name = case["name"].as_str().expect("name");
        let texts = case["texts"].as_object().expect("texts");
        for (surface, value) in texts {
            let text = value.as_str().expect("text");
            if text.is_empty() {
                continue;
            }
            assert!(
                raw_or_http(text).is_none(),
                "{name} {surface} shows a raw error or an HTTP code ({})",
                raw_or_http(text).unwrap_or("?"),
            );
            if text.contains("Refused") {
                saw_refused = true;
            }
            if text.contains("thread:transition") && text.contains("Tokens") {
                saw_capability = true;
            }
            if text.contains("The server hit an error") {
                saw_server = true;
            }
            if text.contains("Not found") {
                saw_not_found = true;
            }
            if text.to_lowercase().contains("reach the server") {
                saw_unreachable = true;
            }
        }
    }
    assert!(
        saw_refused,
        "a 409 is the sentence Refused, not the problem document"
    );
    assert!(
        saw_capability,
        "a 403 names the missing capability and where to get it"
    );
    assert!(saw_server, "a 500 is a sentence, not the server body");
    assert!(
        saw_not_found,
        "a missing attachment is a sentence, not HTTP 404"
    );
    assert!(
        saw_unreachable,
        "a fetch that throws is a sentence, not TypeError"
    );
    let messages = cases
        .iter()
        .find(|case| case["name"] == "messages-409")
        .expect("message list");
    assert_eq!(
        messages["texts"]["messages"].as_str(),
        Some("Refused"),
        "the thread shows the humanError sentence and nothing after it"
    );
    let tools = &report["tools"];
    assert!(
        tools["out"].as_str().unwrap_or("").contains("HTTP 500"),
        "pre#out may keep JSON"
    );
    assert!(
        tools["live"].as_str().unwrap_or("").contains('{'),
        "#live-feed may keep JSON"
    );
    let five = tools["five"].as_str().unwrap_or("");
    assert!(
        raw_or_http(five).is_none(),
        "JSON left in the tools did not stay off the board, the row, the thread, and the toast ({five:?})"
    );
}

fn raw_or_http(text: &str) -> Option<&'static str> {
    if text.contains("HTTP") {
        return Some("HTTP");
    }
    if text.contains('{') || text.contains('}') {
        return Some("JSON");
    }
    if text.contains(" — ") {
        return Some("server detail");
    }
    let lower = text.to_ascii_lowercase();
    for needle in [
        "typeerror",
        "failed to fetch",
        "thread is not in review",
        "panic:",
        "see stack",
        "boom",
        "sql error",
    ] {
        if lower.contains(needle) {
            return Some(needle);
        }
    }
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + 2 < bytes.len() {
        let digit = bytes[i].is_ascii_digit();
        let start = i == 0 || !bytes[i - 1].is_ascii_digit();
        if digit && start && bytes[i + 1].is_ascii_digit() && bytes[i + 2].is_ascii_digit() {
            let end = i + 3;
            let end_ok = end == bytes.len() || !bytes[end].is_ascii_digit();
            if end_ok {
                let n = (bytes[i] - b'0') as u16 * 100
                    + (bytes[i + 1] - b'0') as u16 * 10
                    + (bytes[i + 2] - b'0') as u16;
                if (100..600).contains(&n) {
                    return Some("status code");
                }
            }
        }
        i += 1;
    }
    None
}

fn person_facing_errors() -> serde_json::Value {
    let payload = serde_json::json!({ "html": HTML, "js": script(HTML), "css": UI_CSS });
    let mut child = std::process::Command::new("node")
        .arg("-e")
        .arg(ERROR_SURFACE_HARNESS)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap_or_else(|err| panic!("node is required to run the error surfaces: {err}"));
    serde_json::to_writer(child.stdin.take().expect("stdin"), &payload).expect("write html");
    let out = child.wait_with_output().expect("node");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "error surface harness failed\n{stderr}\n{stdout}"
    );
    serde_json::from_str(&stdout)
        .unwrap_or_else(|err| panic!("error surface harness returned {err}: {stdout}"))
}

fn is_one_sentence(text: &str) -> bool {
    let prose = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let marks = prose
        .chars()
        .filter(|c| matches!(*c, '.' | '!' | '?'))
        .count();
    marks == 1
        && prose.len() > 1
        && (prose.ends_with('.') || prose.ends_with('!') || prose.ends_with('?'))
}

struct EmptyBlock {
    tag: String,
    text: String,
    class_name: String,
}

struct EmptyBoardView {
    id: String,
    first_run_hidden: bool,
    blocks: Vec<EmptyBlock>,
    lanes: usize,
    empty_lanes: usize,
}

impl EmptyBoardView {
    fn label(&self) -> String {
        format!(
            "{} (first-run {})",
            self.id,
            if self.first_run_hidden {
                "hidden"
            } else {
                "visible"
            }
        )
    }
}

fn empty_boards_in_page() -> Vec<EmptyBoardView> {
    let payload = serde_json::json!({ "html": HTML, "js": script(HTML), "css": UI_CSS });
    let mut child = std::process::Command::new("node")
        .arg("-e")
        .arg(EMPTY_BOARD_HARNESS)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap_or_else(|err| panic!("node is required to run the empty board: {err}"));
    serde_json::to_writer(child.stdin.take().expect("stdin"), &payload).expect("write html");
    let out = child.wait_with_output().expect("node");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "empty board harness failed\n{stderr}\n{stdout}"
    );
    let value: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|err| panic!("empty board harness returned {err}: {stdout}"));
    value
        .as_array()
        .expect("board list")
        .iter()
        .map(|scene| EmptyBoardView {
            id: scene["id"].as_str().expect("id").to_string(),
            first_run_hidden: scene["first_run_hidden"]
                .as_bool()
                .expect("first_run_hidden"),
            lanes: scene["lanes"].as_u64().expect("lanes") as usize,
            empty_lanes: scene["empty_lanes"].as_u64().expect("empty_lanes") as usize,
            blocks: scene["blocks"]
                .as_array()
                .expect("blocks")
                .iter()
                .map(|block| EmptyBlock {
                    tag: block["tag"].as_str().expect("tag").to_string(),
                    text: block["text"].as_str().expect("text").to_string(),
                    class_name: block["class_name"]
                        .as_str()
                        .expect("class_name")
                        .to_string(),
                })
                .collect(),
        })
        .collect()
}

const EMPTY_BOARD_HARNESS: &str = r#####"
const fs = require("fs");
const input = JSON.parse(fs.readFileSync(0, "utf8"));
const html = input.html;
const js = input.js;
function functionSource(name) {
  const mark = "\n      function " + name + "(";
  const i = js.indexOf(mark);
  if (i < 0) throw new Error("missing " + name);
  const at = i + 1;
  const open = js.indexOf("{", at);
  let depth = 0;
  for (let j = open; j < js.length; j++) {
    const c = js[j];
    if (c === "{") depth++;
    else if (c === "}") {
      depth--;
      if (depth === 0) return js.slice(at, j + 1);
    }
  }
  throw new Error("unclosed " + name);
}
function makeEl(tag) {
  const el = {
    tag: tag,
    id: "",
    hidden: false,
    children: [],
    className: "",
    textContent: "",
    attrs: {},
  };
  el.appendChild = (child) => { el.children.push(child); return child; };
  el.append = (...kids) => {
    for (const kid of kids) {
      if (typeof kid === "string") {
        const text = makeEl("#text");
        text.textContent = kid;
        el.children.push(text);
      } else {
        el.children.push(kid);
      }
    }
  };
  el.replaceChildren = (...kids) => {
    el.children = [];
    if (kids.length) el.append(...kids);
  };
  el.setAttribute = (key, value) => {
    el.attrs[key] = String(value);
    if (key === "class") el.className = String(value);
    if (key === "hidden") el.hidden = true;
  };
  el.removeAttribute = (key) => { delete el.attrs[key]; };
  return el;
}
function classHas(el, name) {
  return (" " + String(el.className || "") + " ").indexOf(" " + name + " ") >= 0;
}
function ownText(el) {
  const raw = !el.children || el.children.length === 0
    ? String(el.textContent || "")
    : el.children.filter((child) => child.tag === "#text").map((child) => child.textContent).join("");
  return raw.replace(/\s+/g, " ").trim();
}
function describe(root) {
  const blocks = [];
  let lanes = 0;
  let emptyLanes = 0;
  function walk(el) {
    if (!el || el.tag === "#text") return;
    if (classHas(el, "board-col")) lanes++;
    if (classHas(el, "board-empty")) emptyLanes++;
    const text = ownText(el);
    if (text) blocks.push({ tag: el.tag, text: text, class_name: el.className || "" });
    for (const child of el.children || []) walk(child);
  }
  walk(root);
  return { blocks: blocks, lanes: lanes, empty_lanes: emptyLanes };
}
function parseElementAt(src, i) {
  if (src[i] !== "<" || src[i + 1] === "/") throw new Error("expected an open tag at " + i);
  const gt = src.indexOf(">", i);
  let open = src.slice(i + 1, gt).trim();
  const selfClose = open.endsWith("/");
  if (selfClose) open = open.slice(0, -1).trim();
  const nameEnd = open.search(/\s/);
  const tag = (nameEnd < 0 ? open : open.slice(0, nameEnd)).toLowerCase();
  const attrSrc = nameEnd < 0 ? "" : open.slice(nameEnd);
  const el = makeEl(tag);
  const attrRe = /([^\s=]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'=<>`]+)))?/g;
  let m;
  while ((m = attrRe.exec(attrSrc))) {
    const value = m[2] !== undefined ? m[2] : m[3] !== undefined ? m[3] : m[4] !== undefined ? m[4] : "";
    el.setAttribute(m[1], value);
    if (m[1] === "hidden") el.hidden = true;
  }
  i = gt + 1;
  const voidTag = selfClose || ["br", "img", "input", "meta", "link", "hr"].indexOf(tag) >= 0;
  if (!voidTag) {
    while (i < src.length) {
      if (src.startsWith("</", i)) {
        const close = src.indexOf(">", i);
        i = close + 1;
        break;
      }
      if (src[i] === "<") {
        const parsed = parseElementAt(src, i);
        el.children.push(parsed.el);
        i = parsed.i;
      } else {
        const next = src.indexOf("<", i);
        const text = src.slice(i, next < 0 ? src.length : next);
        i = next < 0 ? src.length : next;
        if (text.length) {
          const node = makeEl("#text");
          node.textContent = text;
          el.children.push(node);
        }
      }
    }
  }
  return { el: el, i: i };
}
function firstRunStartsHidden(src) {
  const m = src.match(/<section\b[^>]*\bid="first-run"[^>]*>/);
  if (!m) throw new Error("no first-run");
  return /(?:\s|^)hidden(?:\s|=|>)/.test(m[0]);
}
const byId = {};
for (const id of ["board", "board-summary", "board-title", "first-run"]) {
  byId[id] = makeEl("div");
  byId[id].id = id;
}
byId["first-run"].hidden = false;
const document = {
  getElementById(id) {
    if (!byId[id]) throw new Error("unexpected element " + id);
    return byId[id];
  },
  createElement(tag) { return makeEl(tag); },
  querySelector() { return null; },
  querySelectorAll() { return []; },
};
let threadsById = new Map();
let lastGates = null;
let selectedChannelName = "Desk";
let selectedThreadId = null;
let boardSeen = new Map();
let selectedChannelId = "ch-1";
function boardRects() { return new Map(); }
function glideCards() {}
function renderTeam() {}
function renderThreadHeader() {}
function openConnect() {}
let views;
try {
  eval(functionSource("emptyChannelHelp") + "\n" + functionSource("renderBoard"));
  const out = [];
  const marker = '<div id="board">';
  const at = html.indexOf(marker);
  if (at < 0) throw new Error("no static board");
  const staticBoard = parseElementAt(html, at).el;
  out.push(Object.assign({ id: "static", first_run_hidden: firstRunStartsHidden(html) }, describe(staticBoard)));
  for (const hidden of [false, true]) {
    byId["first-run"].hidden = hidden;
    const help = emptyChannelHelp();
    out.push(Object.assign({ id: "help", first_run_hidden: hidden }, describe(help)));
  }
  for (const hidden of [false, true]) {
    byId["first-run"].hidden = hidden;
    byId.board.replaceChildren();
    boardSeen = new Map();
    renderBoard([], {});
    out.push(Object.assign({ id: "board", first_run_hidden: hidden }, describe(byId.board)));
  }
  views = out;
} catch (err) {
  console.error(err && err.stack || err);
  process.exit(1);
}
process.stdout.write(JSON.stringify(views));
"#####;

const ERROR_SURFACE_HARNESS: &str = r#####"
(function () {
const fs = require("fs");
const vm = require("vm");
const input = JSON.parse(fs.readFileSync(0, "utf8"));
const html = input.html;
const js = input.js;

const all = [];
function classes(el) {
  return String(el.className || "").split(/\s+/).filter(Boolean);
}
function makeEl(tag) {
  const el = {
    tag: String(tag || "div").toLowerCase(),
    id: "",
    className: "",
    hidden: false,
    value: "",
    href: "",
    title: "",
    type: "",
    placeholder: "",
    method: "",
    action: "",
    disabled: false,
    children: [],
    parentElement: null,
    _text: "",
    innerHTML: "",
    style: { setProperty() {} },
    dataset: {},
    attrs: {},
  };
  el.classList = {
    add(...ns) {
      const s = new Set(classes(el));
      ns.forEach((n) => s.add(n));
      el.className = [...s].join(" ");
    },
    remove(...ns) {
      const s = new Set(classes(el));
      ns.forEach((n) => s.delete(n));
      el.className = [...s].join(" ");
    },
    toggle(n, force) {
      const has = classes(el).includes(n);
      const on = force === undefined ? !has : !!force;
      if (on) el.classList.add(n);
      else el.classList.remove(n);
      return on;
    },
    contains: (n) => classes(el).includes(n),
  };
  el.appendChild = (child) => {
    if (child == null) return child;
    if (child.parentElement) child.remove();
    child.parentElement = el;
    el.children.push(child);
    return child;
  };
  el.append = (...kids) => {
    for (const kid of kids) {
      if (typeof kid === "string") {
        const t = makeEl("#text");
        t._text = kid;
        el.appendChild(t);
      } else el.appendChild(kid);
    }
  };
  el.replaceChildren = (...kids) => {
    el.children.forEach((c) => (c.parentElement = null));
    el.children = [];
    if (kids.length) el.append(...kids);
  };
  el.remove = () => {
    const p = el.parentElement;
    if (!p) return;
    p.children = p.children.filter((c) => c !== el);
    el.parentElement = null;
  };
  el.setAttribute = (k, v) => {
    el.attrs[k] = String(v);
    if (k === "class") el.className = String(v);
    if (k === "hidden") el.hidden = true;
    if (k === "id") el.id = String(v);
  };
  el.getAttribute = (k) => (k in el.attrs ? el.attrs[k] : null);
  el.removeAttribute = (k) => {
    delete el.attrs[k];
    if (k === "hidden") el.hidden = false;
  };
  el.addEventListener = () => {};
  el.focus = () => {};
  el.click = () => {
    if (typeof el.onclick === "function") el.onclick({ stopPropagation() {}, preventDefault() {} });
  };
  el.submit = () => {};
  el.after = (other) => {
    const p = el.parentElement;
    if (!p) return;
    if (other.parentElement) other.remove();
    const i = p.children.indexOf(el);
    other.parentElement = p;
    p.children.splice(i + 1, 0, other);
  };
  el.insertBefore = (node, before) => {
    if (node.parentElement) node.remove();
    node.parentElement = el;
    const i = before ? el.children.indexOf(before) : el.children.length;
    el.children.splice(i < 0 ? el.children.length : i, 0, node);
    return node;
  };
  Object.defineProperty(el, "textContent", {
    get() {
      if (!el.children.length) return el._text || "";
      return el.children.map((c) => c.textContent).join("");
    },
    set(v) {
      el._text = String(v);
      el.children.forEach((c) => (c.parentElement = null));
      el.children = [];
    },
  });
  Object.defineProperty(el, "firstElementChild", {
    get() {
      return el.children.find((c) => c.tag !== "#text") || null;
    },
  });
  el.matches = (sel) => sel.split(",").some((part) => matchSimple(el, part.trim()));
  el.querySelector = (sel) => query(el, sel)[0] || null;
  el.querySelectorAll = (sel) => query(el, sel);
  all.push(el);
  return el;
}
function matchSimple(el, sel) {
  if (!sel) return false;
  if (sel.includes(" ")) return false;
  let ok = true;
  const chunks = sel.match(/([#.]?[\w-]+|\[[^\]]+\])/g) || [];
  if (!chunks.length) return false;
  for (const c of chunks) {
    if (c[0] === "#") ok = ok && el.id === c.slice(1);
    else if (c[0] === ".") ok = ok && classes(el).includes(c.slice(1));
    else if (c[0] === "[") {
      const m = c.slice(1, -1);
      const eq = m.indexOf("=");
      if (eq < 0) ok = ok && (m in el.attrs || el.getAttribute(m) != null);
      else {
        const k = m.slice(0, eq);
        let v = m.slice(eq + 1).replace(/^["']|["']$/g, "");
        ok = ok && String(el.attrs[k] || "") === v;
      }
    } else ok = ok && el.tag === c.toLowerCase();
  }
  return ok;
}
function descendants(root) {
  const out = [];
  const walk = (el) => {
    for (const c of el.children || []) {
      out.push(c);
      walk(c);
    }
  };
  walk(root);
  return out;
}
function query(root, sel) {
  const scope = root ? descendants(root) : all;
  const groups = sel.split(",").map((s) => s.trim()).filter(Boolean);
  const found = [];
  for (const group of groups) {
    const parts = group.split(/\s+/);
    let pool = scope;
    for (const part of parts) {
      pool = pool.filter((el) => matchSimple(el, part));
    }
    for (const el of pool) if (!found.includes(el)) found.push(el);
  }
  return found;
}

const byId = new Map();
function getById(id) {
  if (!byId.has(id)) {
    const el = makeEl("div");
    el.id = id;
    byId.set(id, el);
  }
  return byId.get(id);
}
const brand = makeEl("svg");
brand.className = "brand-mark";
brand.innerHTML = '<path fill="currentColor"/>';
const aside = makeEl("aside");
const body = makeEl("body");
const documentStub = {
  title: "",
  body,
  visibilityState: "hidden",
  getElementById: getById,
  createElement: makeEl,
  querySelector: (sel) => query(null, sel)[0] || null,
  querySelectorAll: (sel) => query(null, sel),
  addEventListener() {},
};
const storage = new Map();
const localStorageStub = {
  getItem: (k) => (storage.has(k) ? storage.get(k) : null),
  setItem: (k, v) => storage.set(k, String(v)),
  removeItem: (k) => storage.delete(k),
};
function httpResponse(status, body) {
  const text = typeof body === "string" ? body : JSON.stringify(body);
  return {
    ok: status >= 200 && status < 300,
    status,
    async text() { return text; },
    async json() { return JSON.parse(text || "null"); },
    async arrayBuffer() { return new ArrayBuffer(0); },
  };
}
let fetchImpl = async () => httpResponse(401, "");
const sandboxFetch = (...args) => fetchImpl(...args);

global.document = documentStub;
global.window = {
  location: { origin: "http://maidan.test", href: "http://maidan.test/ui/" },
  addEventListener() {},
  matchMedia() { return { matches: false, addEventListener() {}, removeEventListener() {} }; },
};
global.localStorage = localStorageStub;
global.fetch = sandboxFetch;
global.setInterval = () => 0;
global.clearInterval = () => {};
global.alert = () => { throw new Error("alert"); };
global.confirm = () => false;
global.navigator = { platform: "Linux", userAgent: "node", clipboard: { writeText: async () => {} } };

const extra = `
globalThis.__api = {
  setSignedIn() {
    sessionMemberId = "member-1";
    tokenSession = false;
    document.getElementById("token").value = "";
  },
  setChannel(id, name) {
    selectedChannelId = id;
    selectedChannelName = name;
  },
  markBoardShown() { boardShownFor = selectedChannelId; },
  setThread(id) { selectedThreadId = id; },
  loadThreads, loadMessages, showRowError, responseError, humanError, artifactCard,
  async review(id) {
    const out = await submitReview(id, "approve");
    const li = document.createElement("li");
    showRowError(li, out.why);
    return li;
  },
  async close(id) {
    const out = await closeThread(id);
    const li = document.createElement("li");
    showRowError(li, out.why);
    return li;
  },
  async gate(id) {
    const out = await answerGate(id, "accept", "state-1");
    const li = document.createElement("li");
    showRowError(li, out.why);
    return li;
  },
  async toastFrom(res, prefix) {
    showError(await responseError(res, prefix));
  },
  async download(sha) {
    const card = artifactCard(sha);
    document.getElementById("message-list").appendChild(card);
    const btn = card.querySelector("button");
    await btn.onclick({ stopPropagation() {}, preventDefault() {} });
  },
  clear() {
    for (const id of ["toasts", "board", "board-summary", "message-list", "status", "out", "live-feed"]) {
      const el = document.getElementById(id);
      el.textContent = "";
    }
  },
};
`;
process.on("unhandledRejection", (err) => {
  console.error("UNHANDLED", err && err.stack || err);
  process.exit(1);
});
vm.runInThisContext(js + "\n" + extra, { filename: "index.html" });

function leafText(el) {
  if (!el || el.hidden) return "";
  if (!el.children || el.children.length === 0) return String(el._text || "");
  return el.children.map(leafText).filter(Boolean).join(" ");
}
function surface(id) {
  return leafText(document.getElementById(id)).replace(/\s+/g, " ").trim();
}

(async () => {
  await new Promise((r) => setTimeout(r, 40));
  const api = global.__api;
  api.setSignedIn();
  api.setChannel("ch-1", "general");
  const problem = JSON.stringify({
    type: "about:blank",
    title: "Conflict",
    status: 409,
    detail: 'thread is not in review {"error":"boom"} stack at /tmp/maidan',
  });
  const raw500 = 'panic: sql error at line 12 {code:500}';
  const cases = [];
  function push(name, extraText) {
    const texts = {
      toasts: surface("toasts"),
      board: surface("board"),
      note: surface("board-summary"),
      messages: surface("message-list"),
      row: extraText ? leafText(extraText).replace(/\s+/g, " ").trim() : "",
    };
    cases.push({ name, texts });
    api.clear();
  }

  fetchImpl = async () => httpResponse(409, problem);
  await api.toastFrom(httpResponse(409, problem), "Could not post");
  push("toast-409");

  await api.loadThreads();
  push("board-409");

  api.markBoardShown();
  await api.loadThreads();
  push("note-409");

  api.setThread("th-1");
  await api.loadMessages();
  push("messages-409");

  push("row-review-409", await api.review("th-1"));
  push("row-close-409", await api.close("th-1"));
  push("row-gate-409", await api.gate("g-1"));

  fetchImpl = async () => { throw new TypeError("Failed to fetch"); };
  await api.loadThreads();
  push("board-network");
  api.setThread("th-1");
  await api.loadMessages();
  push("messages-network");
  push("row-review-network", await api.review("th-1"));
  push("row-close-network", await api.close("th-1"));
  push("row-gate-network", await api.gate("g-1"));

  fetchImpl = async (url) => {
    const u = String(url);
    if (u.includes("/meta")) {
      return httpResponse(200, { filename: "notes.txt", size_bytes: 12, mime_type: "text/plain" });
    }
    if (u.includes("/artifacts/")) return httpResponse(404, problem);
    return httpResponse(401, "");
  };
  await api.download("abc123");
  // meta then download is async; the click awaits blob
  push("download-404");

  fetchImpl = async (url) => {
    const u = String(url);
    if (u.includes("/meta")) {
      return httpResponse(200, { filename: "notes.txt", size_bytes: 12, mime_type: "text/plain" });
    }
    if (u.includes("/artifacts/")) throw new TypeError("Failed to fetch");
    return httpResponse(401, "");
  };
  await api.download("abc123");
  push("download-network");

  fetchImpl = async () => httpResponse(500, raw500);
  await api.toastFrom(httpResponse(500, raw500), "Could not load approvals");
  push("toast-500");
  await api.loadThreads();
  push("board-500");

  const forbidden = JSON.stringify({
    detail: 'caller needs thread:transition; see stack {"error":"boom"} HTTP 403',
  });
  fetchImpl = async () => httpResponse(403, forbidden);
  await api.toastFrom(httpResponse(403, forbidden), "Could not post");
  push("toast-403");

  // Tools may keep JSON. These must not be required to be clean, and must not leak onto the five surfaces.
  fetchImpl = async () => httpResponse(401, "");
  setOut({ error: "HTTP 500", detail: "{raw}" });
  appendLive('event {"status":409}');
  const tools = { out: surface("out"), live: surface("live-feed"), five: [surface("toasts"), surface("board"), surface("board-summary"), surface("message-list")].join(" ") };
  console.log(JSON.stringify({ cases, tools }, null, 2));
})().catch((err) => {
  console.error(err && err.stack || err);
  process.exit(1);
});
})();
"#####;

/// Identity is type, not a chip. From docs/UI Design.md: `#identity-pill`
/// loses the pill border, the 999px radius, and the chip background. It is
/// the name and the workspace name, muted, with `#conn-edit` as a ghost.
///
/// This runs the page's `renderIdentity` and applies the page's styles.
/// It fails if that line is shown as a chip, or if it shows anything other
/// than the name and the workspace name.
#[test]
fn ui_js_identity_is_a_type_not_a_chip() {
    let shown = identity_as_shown();
    let name = "Ada Lovelace";
    let workspace = "Northwind";
    assert_eq!(
        shown.texts,
        vec![name.to_string(), workspace.to_string()],
        "identity is the name and the workspace name"
    );
    assert!(
        !shown.texts.iter().any(|text| text.contains("mem_ada")),
        "a member id is a tooltip, not the label"
    );
    for part in &shown.parts {
        if part.id == "conn-edit" {
            continue;
        }
        if let Some(why) = chip_chrome(part) {
            panic!(
                "identity is shown as a chip ({why}) on <{} {} {}>",
                part.tag, part.id, part.class_name
            );
        }
    }
    let name_el = shown
        .parts
        .iter()
        .find(|part| part.text == name)
        .expect("the name");
    let workspace_el = shown
        .parts
        .iter()
        .find(|part| part.id == "identity-ws")
        .expect("the workspace name");
    assert!(
        is_muted(&name_el.color),
        "the name is muted type, got {}",
        name_el.color
    );
    assert!(
        is_muted(&workspace_el.color),
        "the workspace name is muted type, got {}",
        workspace_el.color
    );
    let person = shown
        .parts
        .iter()
        .find(|part| {
            part.class_name
                .split_whitespace()
                .any(|class| class == "person")
        })
        .expect("the name comes from personEl");
    assert_eq!(
        person.title, "mem_ada",
        "the member id stays on the tooltip"
    );
    let button = shown
        .parts
        .iter()
        .find(|part| part.id == "conn-edit")
        .expect("#conn-edit");
    let classes = button.class_name.split_whitespace().collect::<Vec<_>>();
    assert!(
        classes.contains(&"ghost") && !classes.contains(&"primary"),
        "#conn-edit is a ghost, got {}",
        button.class_name
    );
    assert!(
        is_clear(&button.background_color),
        "#conn-edit background is not a ghost ({})",
        button.background_color
    );
    assert!(
        is_clear(&button.border_color),
        "#conn-edit border is not a ghost ({})",
        button.border_color
    );
    let pill = shown
        .parts
        .iter()
        .find(|part| part.id == "identity-pill")
        .expect("#identity-pill");
    assert_ne!(pill.display, "none", "identity is shown");
}

fn chip_chrome(part: &IdentityPart) -> Option<&'static str> {
    if part.border_radius.replace(' ', "").contains("999") {
        return Some("999px radius");
    }
    if visible_border(part) {
        return Some("pill border");
    }
    if !is_clear(&part.background_color) {
        return Some("chip background");
    }
    None
}

fn visible_border(part: &IdentityPart) -> bool {
    let width = part
        .border_width
        .trim()
        .trim_end_matches("px")
        .parse::<f64>()
        .unwrap_or(0.0);
    let style = part.border_style.trim();
    width > 0.0 && style != "none" && style != "hidden" && !is_clear(&part.border_color)
}

fn is_clear(color: &str) -> bool {
    let color = color
        .split_whitespace()
        .collect::<String>()
        .to_ascii_lowercase();
    color.is_empty() || color == "transparent" || color == "none" || color == "rgba(0,0,0,0)"
}

fn is_muted(color: &str) -> bool {
    let color = color
        .split_whitespace()
        .collect::<String>()
        .to_ascii_lowercase();
    color == "#63636c" || color == "rgb(99,99,108)"
}

struct IdentityPart {
    tag: String,
    id: String,
    class_name: String,
    text: String,
    title: String,
    color: String,
    background_color: String,
    border_width: String,
    border_style: String,
    border_color: String,
    border_radius: String,
    display: String,
}

struct IdentityShown {
    texts: Vec<String>,
    parts: Vec<IdentityPart>,
}

fn identity_as_shown() -> IdentityShown {
    let payload = serde_json::json!({ "html": HTML, "js": script(HTML), "css": UI_CSS });
    let mut child = std::process::Command::new("node")
        .arg("-e")
        .arg(IDENTITY_HARNESS)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap_or_else(|err| panic!("node is required to show identity: {err}"));
    serde_json::to_writer(child.stdin.take().expect("stdin"), &payload).expect("write html");
    let out = child.wait_with_output().expect("node");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "identity harness failed\n{stderr}\n{stdout}"
    );
    let value: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|err| panic!("identity harness returned {err}: {stdout}"));
    let part = |value: &serde_json::Value| IdentityPart {
        tag: value["tag"].as_str().unwrap_or("").to_string(),
        id: value["id"].as_str().unwrap_or("").to_string(),
        class_name: value["class_name"].as_str().unwrap_or("").to_string(),
        text: value["text"].as_str().unwrap_or("").to_string(),
        title: value["title"].as_str().unwrap_or("").to_string(),
        color: value["color"].as_str().unwrap_or("").to_string(),
        background_color: value["background_color"].as_str().unwrap_or("").to_string(),
        border_width: value["border_width"].as_str().unwrap_or("").to_string(),
        border_style: value["border_style"].as_str().unwrap_or("").to_string(),
        border_color: value["border_color"].as_str().unwrap_or("").to_string(),
        border_radius: value["border_radius"].as_str().unwrap_or("").to_string(),
        display: value["display"].as_str().unwrap_or("").to_string(),
    };
    IdentityShown {
        texts: value["texts"]
            .as_array()
            .expect("texts")
            .iter()
            .map(|text| text.as_str().expect("text").to_string())
            .collect(),
        parts: value["parts"]
            .as_array()
            .expect("parts")
            .iter()
            .map(part)
            .collect(),
    }
}

const IDENTITY_HARNESS: &str = r####"
const fs = require("fs");
const input = JSON.parse(fs.readFileSync(0, "utf8"));
const html = input.html;
const js = input.js;
function functionSource(name) {
  const key = "function " + name + "(";
  let at = js.indexOf(key);
  if (at < 0) throw new Error("missing " + name);
  if (js.slice(at - 6, at) === "async ") at -= 6;
  const parenAt = js.indexOf("(", at);
  let paren = 0;
  let sig = parenAt;
  for (; sig < js.length; sig++) {
    if (js[sig] === "(") paren++;
    else if (js[sig] === ")") {
      paren--;
      if (paren === 0) { sig++; break; }
    }
  }
  const open = js.indexOf("{", sig);
  let depth = 0;
  for (let j = open; j < js.length; j++) {
    const c = js[j];
    if (c === "{") depth++;
    else if (c === "}") {
      depth--;
      if (depth === 0) return js.slice(at, j + 1);
    }
  }
  throw new Error("unclosed " + name);
}
function makeEl(tag) {
  const el = {
    tag: tag,
    id: "",
    hidden: false,
    className: "",
    textContent: "",
    title: "",
    children: [],
    parent: null,
    style: {},
    attrs: {},
  };
  el.appendChild = (child) => {
    child.parent = el;
    el.children.push(child);
    return child;
  };
  el.append = (...kids) => {
    for (const kid of kids) {
      if (typeof kid === "string") {
        const text = makeEl("#text");
        text.textContent = kid;
        el.appendChild(text);
      } else el.appendChild(kid);
    }
  };
  el.replaceChildren = (...kids) => {
    el.children = [];
    if (kids.length) el.append(...kids);
  };
  el.setAttribute = (key, value) => {
    el.attrs[key] = String(value);
    if (key === "id") el.id = String(value);
    if (key === "class") el.className = String(value);
    if (key === "hidden") el.hidden = true;
  };
  el.removeAttribute = (key) => {
    delete el.attrs[key];
    if (key === "hidden") el.hidden = false;
  };
  return el;
}
function walk(el, fn) {
  fn(el);
  for (const child of el.children) walk(child, fn);
}
function byId(root, id) {
  let found = null;
  walk(root, (el) => {
    if (el.id === id) found = el;
  });
  return found;
}
const css = String(input.css || "").replace(/\/\*[\s\S]*?\*\//g, "");
function skipAt(css, i) {
  const brace = css.indexOf("{", i);
  const semi = css.indexOf(";", i);
  if (brace < 0 || (semi >= 0 && semi < brace)) return semi < 0 ? css.length : semi + 1;
  let depth = 0;
  for (let j = brace; j < css.length; j++) {
    if (css[j] === "{") depth++;
    else if (css[j] === "}") {
      depth--;
      if (depth === 0) return j + 1;
    }
  }
  return css.length;
}
const rules = [];
const vars = {};
{
  let i = 0;
  while (i < css.length) {
    while (i < css.length && /\s/.test(css[i])) i++;
    if (i >= css.length) break;
    if (css[i] === "@") {
      i = skipAt(css, i);
      continue;
    }
    const brace = css.indexOf("{", i);
    if (brace < 0) break;
    const selector = css.slice(i, brace).trim();
    let depth = 0;
    let j = brace;
    for (; j < css.length; j++) {
      if (css[j] === "{") depth++;
      else if (css[j] === "}") {
        depth--;
        if (depth === 0) { j++; break; }
      }
    }
    const body = css.slice(brace + 1, j - 1);
    const decls = {};
    for (const part of body.split(";")) {
      const c = part.indexOf(":");
      if (c < 0) continue;
      const k = part.slice(0, c).trim().toLowerCase();
      const v = part.slice(c + 1).trim();
      if (k) decls[k] = v;
    }
    if (selector === ":root") {
      for (const [k, v] of Object.entries(decls)) if (k.startsWith("--")) vars[k] = v;
    }
    for (const sel of selector.split(",").map((s) => s.trim()).filter(Boolean)) {
      if (!sel.startsWith("@")) rules.push({ selector: sel, decls });
    }
    i = j;
  }
}
function subVars(value) {
  let v = value;
  for (let n = 0; n < 4; n++) {
    const next = v.replace(/var\(\s*(--[A-Za-z0-9_-]+)\s*\)/g, (_, name) => vars[name] || "");
    if (next === v) break;
    v = next;
  }
  return v.trim();
}
function expand(decls) {
  const out = Object.assign({}, decls);
  if (out.border) {
    const b = out.border.trim();
    if (b === "0" || b === "none" || b === "0px") {
      out["border-width"] = "0";
      out["border-style"] = "none";
      out["border-color"] = "transparent";
    } else {
      const m = b.match(/^(\S+)\s+(\S+)\s+(.+)$/);
      if (m) {
        out["border-width"] = m[1];
        out["border-style"] = m[2];
        out["border-color"] = m[3];
      }
    }
    delete out.border;
  }
  if (out.background) {
    const b = out.background.trim().toLowerCase();
    if (b === "none" || b === "transparent") out["background-color"] = "transparent";
    else if (!b.includes("url(")) out["background-color"] = out.background.trim();
    delete out.background;
  }
  return out;
}
function specificity(selector) {
  const ids = (selector.match(/#[A-Za-z0-9_-]+/g) || []).length;
  const cls = (selector.match(/(\.[A-Za-z0-9_-]+|\[[^\]]+\])/g) || []).length;
  const noId = selector.replace(/#[A-Za-z0-9_-]+/g, " ");
  const noAttr = noId.replace(/\[[^\]]+\]/g, " ");
  const tags = (noAttr.match(/[A-Za-z][A-Za-z0-9_-]*/g) || []).filter((t) => t !== "not").length;
  return ids * 100 + cls * 10 + tags;
}
function matchCompound(el, compound) {
  const m = compound.match(/^([A-Za-z][A-Za-z0-9_-]*)?(#[A-Za-z0-9_-]+)?((?:\.[A-Za-z0-9_-]+)*)(\[[^\]]+\])?$/);
  if (!m) return false;
  const tag = m[1] || "";
  const id = m[2] ? m[2].slice(1) : "";
  const classes = m[3] ? m[3].split(".").filter(Boolean) : [];
  const attr = m[4] || "";
  if (tag && tag.toLowerCase() !== el.tag) return false;
  if (id && id !== el.id) return false;
  const have = new Set(String(el.className || "").split(/\s+/).filter(Boolean));
  for (const c of classes) if (!have.has(c)) return false;
  if (attr === "[hidden]" && !el.hidden) return false;
  if (attr && attr !== "[hidden]") return false;
  return Boolean(tag || id || classes.length || attr);
}
function matches(el, selector) {
  const parts = selector.split(/\s+/).filter(Boolean);
  if (!parts.length) return false;
  let node = el;
  for (let i = parts.length - 1; i >= 0; i--) {
    let found = false;
    while (node) {
      if (matchCompound(node, parts[i])) {
        found = true;
        node = node.parent;
        break;
      }
      if (i === parts.length - 1) return false;
      node = node.parent;
    }
    if (!found) return false;
  }
  return true;
}
function specified(el) {
  const acc = {};
  const rank = {};
  rules.forEach((rule, order) => {
    if (!matches(el, rule.selector)) return;
    const spec = specificity(rule.selector) * 100000 + order;
    const decls = expand(rule.decls);
    for (const [k, v] of Object.entries(decls)) {
      if (rank[k] == null || spec >= rank[k]) {
        rank[k] = spec;
        acc[k] = v;
      }
    }
  });
  for (const [k, v] of Object.entries(el.style || {})) {
    acc[k] = v;
  }
  return acc;
}
const inherited = new Set(["color", "font-weight", "font-size"]);
function resolved(el, prop) {
  const own = specified(el)[prop];
  if (own && own !== "inherit" && own !== "unset") return subVars(own);
  if ((own === "inherit" || own == null) && inherited.has(prop) && el.parent) return resolved(el.parent, prop);
  if (prop === "color") return "#232327";
  if (prop === "font-weight") return "400";
  if (prop === "font-size") return "14px";
  if (prop === "background-color") return "transparent";
  if (prop === "border-width") return "0";
  if (prop === "border-style") return "none";
  if (prop === "border-color") return "transparent";
  if (prop === "border-radius") return "0";
  if (prop === "display") return "inline";
  return "";
}
function ownText(el) {
  if (!el.children.length) return String(el.textContent || "").replace(/\s+/g, " ").trim();
  return el.children.filter((c) => c.tag === "#text").map((c) => c.textContent).join("").replace(/\s+/g, " ").trim();
}
function sliceElement(src, marker) {
  const at = src.indexOf(marker);
  if (at < 0) throw new Error("missing " + marker);
  let i = at;
  let depth = 0;
  while (i < src.length) {
    if (src.startsWith("<!--", i)) {
      i = src.indexOf("-->", i) + 3;
      continue;
    }
    if (src[i] !== "<") { i++; continue; }
    const gt = src.indexOf(">", i);
    const raw = src.slice(i, gt + 1);
    const closing = raw.startsWith("</");
    const self = /\/>$/.test(raw) || /^<(input|br|img|meta|link)\b/i.test(raw);
    if (closing) depth--;
    else if (!self) depth++;
    i = gt + 1;
    if (depth === 0) return src.slice(at, i);
  }
  throw new Error("unclosed " + marker);
}
function parseElement(src) {
  const root = makeEl("#frag");
  const stack = [root];
  const re = /<!--[\s\S]*?-->|<\/([A-Za-z0-9]+)>|<([A-Za-z0-9]+)([^>]*?)(\/?)>/g;
  let m;
  let last = 0;
  while ((m = re.exec(src))) {
    const text = src.slice(last, m.index).replace(/\s+/g, " ");
    if (text.trim()) {
      const t = makeEl("#text");
      t.textContent = text.trim();
      stack[stack.length - 1].appendChild(t);
    }
    last = m.index + m[0].length;
    if (m[0].startsWith("<!--")) continue;
    if (m[1]) {
      const name = m[1].toLowerCase();
      while (stack.length > 1 && stack[stack.length - 1].tag !== name) stack.pop();
      if (stack.length > 1) stack.pop();
      continue;
    }
    const el = makeEl(m[2].toLowerCase());
    const attrRe = /([A-Za-z0-9:-]+)(?:\s*=\s*"([^"]*)"|\s*=\s*'([^']*)')?/g;
    let a;
    const attrSrc = m[3] || "";
    while ((a = attrRe.exec(attrSrc))) {
      const key = a[1].toLowerCase();
      const val = a[2] != null ? a[2] : a[3] != null ? a[3] : "";
      if (key === "class") el.className = val;
      else if (key === "id") el.id = val;
      else if (key === "title") el.title = val;
      else if (key === "hidden") el.hidden = true;
      el.attrs[key] = val;
    }
    stack[stack.length - 1].appendChild(el);
    const self = m[4] === "/" || ["input", "br", "img", "meta", "link"].includes(el.tag);
    if (!self) stack.push(el);
  }
  if (root.children.length !== 1) throw new Error("identity markup did not parse to one element");
  return root.children[0];
}
const pill = parseElement(sliceElement(html, '<span id="identity-pill"'));
const root = makeEl("header");
root.appendChild(pill);
const workspace = makeEl("input");
workspace.id = "workspace";
workspace.value = "0123456789abcdef";
root.appendChild(workspace);
const status = makeEl("p");
status.id = "session-status";
root.appendChild(status);
const document = {
  getElementById(id) { return byId(root, id); },
  createElement(tag) { return makeEl(tag); },
};
const memberDirectory = new Map([
  ["mem_ada", { name: "Ada Lovelace", kind: "human" }],
]);
const fetch = async () => ({
  ok: true,
  json: async () => ({ name: "Northwind" }),
});
const renderIdentity = new Function(
  "document",
  "fetch",
  "memberDirectory",
  "sessionMemberId",
  functionSource("memberName") + "\n" +
    functionSource("memberKind") + "\n" +
    functionSource("initials") + "\n" +
    functionSource("hueFor") + "\n" +
    functionSource("avatarEl") + "\n" +
    functionSource("personEl") + "\n" +
    "function wid() { return document.getElementById('workspace').value.trim(); }\n" +
    "function token() { return false; }\n" +
    "function headers() { return {}; }\n" +
    "function apiReadPath(suffix) { return suffix; }\n" +
    "function api(url, options) { return fetch(url, options); }\n" +
    functionSource("renderIdentity") + "\n" +
    "return renderIdentity;"
)(document, fetch, memberDirectory, null);
renderIdentity("mem_ada").then(() => {
  pill.hidden = false;
  pill.removeAttribute("hidden");
  const parts = [];
  walk(pill, (el) => {
    if (el.tag === "#text") return;
    parts.push({
      tag: el.tag,
      id: el.id,
      class_name: el.className,
      text: ownText(el),
      title: el.title || "",
      color: resolved(el, "color"),
      font_weight: String(resolved(el, "font-weight")),
      background_color: resolved(el, "background-color"),
      border_width: resolved(el, "border-width"),
      border_style: resolved(el, "border-style"),
      border_color: resolved(el, "border-color"),
      border_radius: resolved(el, "border-radius"),
      display: resolved(el, "display"),
    });
  });
  const texts = [];
  walk(pill, (el) => {
    if (el.id === "conn-edit" || (el.parent && el.parent.id === "conn-edit")) return;
    const t = ownText(el);
    if (t) texts.push(t);
  });
  const button = parts.find((p) => p.id === "conn-edit");
  process.stdout.write(JSON.stringify({ texts, parts, button }, null, 2));
}).catch((err) => {
  console.error(err);
  process.exit(1);
});

"####;

/// The line under the header. A member with a display name sees that name.
/// The member id stays on the title, where a developer can read it, and is
/// the visible line only when no display name is set.
#[test]
fn ui_js_signed_in_line_uses_display_name() {
    let named = signed_in_line("mem_river", Some("River Chen"));
    assert_eq!(named.text, "Signed in · River Chen");
    assert!(
        !named.text.contains("mem_river"),
        "a display name replaces the member id on the signed-in line"
    );
    assert_eq!(named.title, "mem_river");

    let blank = signed_in_line("mem_river", Some("   "));
    assert_eq!(blank.text, "Signed in · mem_river");
    assert_eq!(blank.title, "mem_river");

    let unnamed = signed_in_line("mem_river", None);
    assert_eq!(unnamed.text, "Signed in · mem_river");
    assert_eq!(unnamed.title, "mem_river");
}

struct SignedInLine {
    text: String,
    title: String,
}

fn signed_in_line(member_id: &str, display_name: Option<&str>) -> SignedInLine {
    let js = script(HTML);
    let start = js
        .find("async function refreshSession(")
        .expect("refreshSession");
    let rel = js[start..]
        .find("\n      }\n")
        .expect("end of refreshSession");
    let refresh = format!("{}\n      }}", &js[start..start + rel]);
    let payload = serde_json::json!({
        "refresh": refresh,
        "member_id": member_id,
        "display_name": display_name,
    });
    let mut child = std::process::Command::new("node")
        .arg("-e")
        .arg(SIGNED_IN_HARNESS)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap_or_else(|err| panic!("node is required to show the signed-in line: {err}"));
    serde_json::to_writer(child.stdin.take().expect("stdin"), &payload).expect("write");
    let out = child.wait_with_output().expect("node");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "signed-in harness failed: {}\n{stdout}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value = serde_json::from_str(stdout.trim()).expect("json");
    SignedInLine {
        text: value["text"].as_str().unwrap_or("").to_string(),
        title: value["title"].as_str().unwrap_or("").to_string(),
    }
}

const SIGNED_IN_HARNESS: &str = r####"
const fs = require("fs");
const input = JSON.parse(fs.readFileSync(0, "utf8"));
function makeEl() {
  return { hidden: false, textContent: "", title: "", className: "", value: "" };
}
const els = {
  "session-status": makeEl(),
  mint: makeEl(),
  workspace: makeEl(),
  "token-member": makeEl(),
};
const document = { getElementById(id) { return els[id]; } };
let sessionMemberId = null;
let tokenSession = false;
function base() { return ""; }
function wid() { return "workspace-already-set"; }
const refreshSession = new Function(
  "document",
  "fetch",
  "base",
  "wid",
  "function api(url, options) { return fetch(url, options); }\n" + input.refresh + "\nreturn refreshSession;"
)(document, async () => ({
  ok: true,
  json: async () => ({
    member_id: input.member_id,
    workspace_id: "ws",
    display_name: input.display_name,
    token_id: null,
  }),
}), base, wid);
refreshSession().then(() => {
  const el = els["session-status"];
  process.stdout.write(JSON.stringify({ text: el.textContent, title: el.title }));
}).catch((err) => {
  console.error(err);
  process.exit(1);
});
"####;

/// Naming a workspace goes through PATCH on the workspace resource, the same
/// path the server mounts for a bearer and for a signed-in session.
#[test]
fn ui_js_names_a_workspace_with_patch() {
    let body = function_body(script(HTML), "saveWorkspaceName");
    assert!(
        body.contains("apiWritePath(`/workspaces/${wid()}`)"),
        "the name control must PATCH the workspace path"
    );
    assert!(
        body.contains("method: \"PATCH\""),
        "naming uses PATCH, not a new workspace"
    );
    assert!(!body.contains("POST"), "naming must not create a workspace");
}

/// A keyboard focus reveals the pin control. Hover-only hides it from
/// anyone who is not using a pointer. The reaction control already uses
/// `:focus-within`; the pin toggle follows it.
#[test]
fn ui_js_pin_toggle_shows_on_keyboard_focus() {
    let css = page();
    assert!(
        css.contains("#message-list .msg:hover .pin-toggle, #message-list .msg:focus-within .pin-toggle, .pin-toggle.pinned { opacity: 1; }"),
        "keyboard focus on a message reveals its pin toggle"
    );
    assert!(
        !css.contains("#message-list .msg:hover .pin-toggle, .pin-toggle.pinned { opacity: 1; }"),
        "the pin toggle is not hover-only"
    );
}

/// Inputs, selects, and textareas are 16px. The body is 14px, and `font:
/// inherit` would hand that size to every control. iOS Safari zooms the page
/// when a focused control is smaller than 16px. A later rule must not set a
/// smaller size on one of them.
#[test]
fn ui_js_form_controls_are_16px() {
    let css = UI_CSS;
    // The button rule is `button, input, select, textarea {` on one line.
    // Matching that substring would treat the inherit rule as the 16px rule.
    let shared = css
        .find("input, select, textarea {\n")
        .expect("shared control rule");
    let inherit = css
        .find("button, input, select, textarea { font: inherit; color: inherit; }")
        .expect("controls inherit the body font");
    assert!(
        shared > inherit,
        "16px has to come after font: inherit, or the body size wins"
    );
    let block_end = css[shared..].find('}').expect("rule end") + shared;
    let block = &css[shared..block_end];
    assert!(
        block.contains("font-size: 16px;"),
        "the shared control rule sets 16px, not the body's 14px"
    );
    let mut small = Vec::new();
    for raw in css.split('}') {
        let Some((sel, body)) = raw.split_once('{') else {
            continue;
        };
        if !selector_targets_form_control(sel) || !body.contains("font-size:") {
            continue;
        }
        match declared_font_size_px(body) {
            Some(px) if px >= 16.0 => {}
            other => small.push(format!(
                "{} ({})",
                sel.split_whitespace().next().unwrap_or(sel.trim()),
                other
                    .map(|px| format!("{px}px"))
                    .unwrap_or_else(|| "unparsed".into())
            )),
        }
    }
    assert!(
        small.is_empty(),
        "a form control is under 16px: {}",
        small.join("; ")
    );
}

fn selector_targets_form_control(selector: &str) -> bool {
    selector.split(',').any(|part| {
        let part = part.trim();
        let last = part
            .rsplit([' ', '>', '+', '~'])
            .next()
            .unwrap_or(part)
            .trim();
        if last == "#palette-input" {
            return true;
        }
        let type_name = last.split(['.', '#', ':', '[']).next().unwrap_or(last);
        matches!(type_name, "input" | "select" | "textarea")
    })
}

/// `px` as written. `rem` against the browser default root of 16px, which is
/// what iOS compares. The board sets the body to 14px and does not change the root.
fn declared_font_size_px(body: &str) -> Option<f64> {
    let rest = body.split("font-size:").nth(1)?.trim();
    let value = rest.split([';', '\n']).next()?.trim();
    if let Some(n) = value.strip_suffix("px") {
        return n.trim().parse().ok();
    }
    if let Some(n) = value.strip_suffix("rem") {
        let n: f64 = n.trim().parse().ok()?;
        return Some(n * 16.0);
    }
    None
}

/// Opening a thread from another channel selects that channel. The palette
/// reaches those threads through selectThread, which has to set the sidebar
/// highlight, not only the open thread.
#[test]
fn ui_js_palette_jump_selects_the_threads_channel() {
    let js = script(HTML);
    let select = function_body(js, "selectThread");
    assert!(
        select.contains("focusChannel(channelId)") && select.contains("focusChannel(found)"),
        "selectThread selects the channel a thread lives in"
    );
    let focus = function_body(js, "focusChannel");
    assert!(
        focus.contains("selectedChannelId = channelId")
            && focus.contains("n.classList.toggle(\"selected\", n.dataset.id === channelId)"),
        "the sidebar highlight follows the channel"
    );
    assert!(
        js.contains("selectThread(next.thread_id, title);")
            && js.contains("selectThread(th.id, th.title || th.id)"),
        "the palette opens a thread through selectThread"
    );
}

/// A message click used to fill the collapsed Edit message fields and leave
/// them closed. The pencil sits in the meta row, beside the pin toggle, and
/// both the pencil and the message open that details element.
#[test]
fn ui_js_message_click_opens_the_edit_details() {
    let js = script(HTML);
    let render = function_body(js, "renderMessages");
    assert!(
        render.contains("className = \"edit-toggle\"")
            && render.contains("openMessageEdit(m)")
            && render.contains("meta.appendChild(editBtn)"),
        "a message offers an edit control beside the pin toggle"
    );
    let open = function_body(js, "openMessageEdit");
    assert!(
        open.contains("getElementById(\"edit-message-id\").value = message.id")
            && open.contains("details.open = true"),
        "clicking a message opens the Edit message details"
    );
    let css = page();
    assert!(
        css.contains("#message-list .msg:hover .edit-toggle, #message-list .msg:focus-within .edit-toggle { opacity: 1; }"),
        "the edit control shows on hover and on keyboard focus"
    );
    assert!(
        css.contains("#message-list .msg:hover .pin-toggle, #message-list .msg:focus-within .pin-toggle, .pin-toggle.pinned { opacity: 1; }"),
        "the pin toggle still shows on keyboard focus"
    );
}

/// The first-run card says what a workspace id is and where it comes from.
#[test]
fn ui_js_first_run_says_what_a_workspace_id_is() {
    let first_run = HTML
        .split("id=\"first-run\"")
        .nth(1)
        .and_then(|rest| rest.split("</section>").next())
        .expect("the first-run card");
    assert!(
        first_run.contains("A workspace id is the id of one room on this server.")
            && first_run.contains("maidan init")
            && first_run.contains("prints it when that room is created"),
        "the card says what a workspace id is and where it comes from"
    );
}

/// `.brand-sub` is unused. The marker `data-ui-version` is not a version.
#[test]
fn ui_js_drops_dead_brand_sub_and_the_version_marker() {
    let html = HTML;
    let css = page();
    assert!(
        !html.contains("data-ui-version"),
        "the version marker is gone"
    );
    assert!(
        !css.contains(".brand-sub"),
        "the unused brand-sub rule is gone"
    );
    assert!(
        !html.contains("brand-sub"),
        "the page does not use brand-sub"
    );
}
