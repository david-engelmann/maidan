//! Guard against "function called but never defined" in the `/ui` console JS.
//! The `/ui` is vanilla HTML/JS with no browser in CI, so a
//! reference-to-undefined-function bug (which is exactly what broke the write
//! path — `apiWritePath`/`requireAuthForWrite` were called but never defined)
//! otherwise sails through `cargo test`. This is a dependency-free static
//! check: every *bare* call `ident(` must resolve to a local definition, a
//! function parameter, or a known JS/DOM global.

const HTML: &str = include_str!("../static/index.html");

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$'
}

fn script(html: &str) -> &str {
    let start = html.find("<script>").expect("a <script> block") + "<script>".len();
    let end = html[start..].find("</script>").expect("a </script>") + start;
    &html[start..end]
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
        HTML.contains("data-tab=\"session\""),
        "the Session tab button must exist"
    );
    assert!(
        HTML.contains("id=\"panel-session\""),
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
        HTML.contains("id=\"attenuation-warning\""),
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
        HTML.contains("class=\"skip-link\"") && HTML.contains("id=\"main-content\""),
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
        HTML.contains("id=\"panel-work\"") && HTML.contains("id=\"work-channel\""),
        "the Work panel + channel selector must exist"
    );
    assert!(
        HTML.contains("data-tab=\"work\""),
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
        HTML.contains("id=\"panel-prefs\"") && HTML.contains("data-tab=\"prefs\""),
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
        HTML.contains("id=\"panel-glass\"") && HTML.contains("data-tab=\"glass\""),
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
        HTML.contains("id=\"waiting-list\"") && HTML.contains("id=\"waiting-sla\""),
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
            HTML.contains(color),
            "the UI must retain brand color {color}"
        );
    }
    assert!(
        HTML.contains("<svg class=\"brand-mark\" viewBox=\"0 0 64 64\"")
            && HTML.contains("M38.95,32.85L48.07,37.53L59.65,35.70")
            && HTML.contains("M37.52,27.69L47.28,24.55L51.08,17.60"),
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
        HTML.contains("<pre id=\"live-feed\" hidden>"),
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
        HTML.contains("id=\"needs-you\"") && HTML.contains("id=\"needs-you-list\""),
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
            && html.contains("@media (prefers-reduced-motion: reduce)"),
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
        HTML.contains(r#"<div id="toasts"></div>"#),
        "the toast region"
    );
}

/// The board is the one place a channel's threads are drawn. The sidebar used
/// to list the same threads from the same fetch, so each appeared twice in two
/// visual languages.
#[test]
fn ui_js_draws_a_channels_threads_once_on_the_board() {
    assert!(
        !HTML.contains("id=\"thread-list\""),
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
        !HTML.contains("acts as any member"),
        "the pre-411 impersonation wording is gone"
    );
    let s = script(HTML);
    assert!(
        s.contains("/rotate`") && s.contains("me.token_id"),
        "rotation uses the rotate route and the token id /me returns"
    );
    assert!(
        HTML.contains("id=\"rotate-own-token\"") && HTML.contains("id=\"rotate-token\""),
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

/// The first screen is the board. Live controls sit in a closed menu, a
/// refused subscribe is a status line, an empty Needs you queue is one
/// sentence, and a decision row has a single filled button.
#[test]
fn ui_js_keeps_the_first_screen_quiet() {
    assert!(
        HTML.contains("id=\"live-more\"") && HTML.contains("id=\"needs-you-quiet\""),
        "the live menu and the empty-queue line exist"
    );
    assert!(
        !HTML.contains("class=\"primary\">Connect WS"),
        "connecting the socket is not the page's primary button"
    );
    assert!(
        !HTML.contains("Connect to update the board"),
        "the live hint no longer leads the first screen"
    );
    assert!(
        HTML.contains("Nothing is waiting on you."),
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
        HTML.contains("<details id=\"tools\">"),
        "More tools is on the page"
    );
    assert!(
        !HTML.contains("<details id=\"tools\" open"),
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
        HTML.contains("<section id=\"collab-panel\" hidden>"),
        "the thread panel starts hidden"
    );
    assert!(
        HTML.contains("#collab-panel[hidden] { display: none; }"),
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
        HTML.contains(".board-col { background: transparent; border: 0; box-shadow: none; padding: 0; min-height: 0; }"),
        "a lane has no background, border, padding, or shadow"
    );
    assert!(
        !HTML.contains("#efece5")
            && !HTML.contains(".board-col .dot")
            && !HTML.contains(".board-col h3 .count"),
        "the tinted box, the colored dot, and the white count pill are gone"
    );
    assert!(
        HTML.contains("gap: 16px")
            && HTML.contains(".board-col h3 { margin: 0 0 8px; font-size: 12px; font-weight: 400; letter-spacing: 0; text-transform: none; color: #63636c; }"),
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
        !HTML.contains("legend-box")
            && !HTML.contains("What the badges mean")
            && !HTML.contains("chrome-badge"),
        "the badge legend and the pill class are gone"
    );
    assert!(
        HTML.contains(".card-foot { display: flex; align-items: center; gap: 0.4rem; font-size: 12px; font-weight: 400; color: #63636c; }"),
        "the card foot is 12px muted type"
    );
    assert!(
        HTML.contains("#thread-badge { font-size: 12px; font-weight: 400; color: #63636c; }"),
        "the thread state is the same word, not a pill"
    );
    assert!(
        HTML.contains(
            ".ny-kind { font-size: 12px; font-weight: 400; color: #63636c; white-space: nowrap; }"
        ) && !HTML.contains(".ny-kind.gate")
            && !HTML.contains("#ffedd5"),
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
const start = html.indexOf("<script>") + "<script>".length;
const js = html.slice(start, html.indexOf("</script>", start));
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
        !HTML.contains("title=\"Requires bearer token\""),
        "Add task and new channel no longer say they require a bearer"
    );
    assert!(
        HTML.contains("title=\"Sign in or paste a token\""),
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
