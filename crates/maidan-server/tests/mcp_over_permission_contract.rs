//! Over-permission scan of the MCP tool surface (Open Work later-S).
//!
//! Every tool declares a known capability, that declaration is the narrowest
//! one that covers what the handler does, and the handler does not do more
//! than the declaration allows. Reviewed exceptions are named and pinned so a
//! new tool cannot join them silently. The HTTP twins use the same caps for
//! the privileged reads and the personal-state writes; holder-side token
//! tools act only on the caller.

mod source_scan;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use maidan_auth::capability::{self, is_known};
use maidan_mcp::tools::{is_read_only, required_capability};
use source_scan::without_tests;

/// Capabilities that only let the holder read, search, subscribe or resolve
/// a secret. A write or an authority grant is not a read capability.
const READ_CAPS: &[&str] = &[
    capability::WORKSPACE_READ,
    capability::SEARCH_QUERY,
    capability::EVENT_SUBSCRIBE,
    capability::SECRET_READ,
    capability::AUDIT_READ_GLOBAL,
];

/// Read-only tools whose declared capability is not a read capability.
/// Each listed cap is the narrowest that covers the data, or the actor who
/// can use the result; the HTTP twin uses the same cap.
const PRIVILEGED_READS: &[(&str, &str)] = &[
    // Advice is gated to a token that can act on a land gate.
    ("advise_land_gate", capability::THREAD_TRANSITION),
    // The whole log, including private channels and DMs.
    ("catch_up_events", capability::TOKEN_ADMIN),
    ("list_channel_members", capability::CHANNEL_ADMIN),
    ("list_delegation_grants", capability::TOKEN_ADMIN),
    ("list_frozen_members", capability::TOKEN_ADMIN),
    ("list_secret_egress_hosts", capability::SECRET_ADMIN),
    ("list_share_tickets", capability::TOKEN_ADMIN),
    ("verify_workspace_export", capability::TOKEN_ADMIN),
];

/// Mutating tools that declare a read capability. Personal self-scoped
/// state and holder-side token operations, plus the documented HTTP twins
/// (`POST /workspaces/{wid}/group-dms`, `POST /tokens/attenuate`, secret
/// resolve) that use the same cap.
const READ_CAP_WRITES: &[(&str, &str)] = &[
    ("attenuate_token", capability::WORKSPACE_READ),
    ("delegate_token", capability::WORKSPACE_READ),
    ("delete_member_email", capability::WORKSPACE_READ),
    ("follow_channel", capability::WORKSPACE_READ),
    ("follow_member", capability::WORKSPACE_READ),
    ("follow_thread", capability::WORKSPACE_READ),
    ("mark_all_notifications_read", capability::WORKSPACE_READ),
    ("mark_inbox_read", capability::WORKSPACE_READ),
    ("mark_notification_read", capability::WORKSPACE_READ),
    ("mute_channel", capability::WORKSPACE_READ),
    ("mute_thread", capability::WORKSPACE_READ),
    ("open_group_dm", capability::WORKSPACE_READ),
    ("request_approval", capability::WORKSPACE_READ),
    ("resolve_secret", capability::SECRET_READ),
    ("rotate_token", capability::WORKSPACE_READ),
    ("set_delivery_mode", capability::WORKSPACE_READ),
    ("set_member_email", capability::WORKSPACE_READ),
    ("set_notification_pref", capability::WORKSPACE_READ),
    ("snooze_notification", capability::WORKSPACE_READ),
    ("unfollow_channel", capability::WORKSPACE_READ),
    ("unfollow_member", capability::WORKSPACE_READ),
    ("unfollow_thread", capability::WORKSPACE_READ),
    ("unmute_channel", capability::WORKSPACE_READ),
    ("unmute_thread", capability::WORKSPACE_READ),
];

/// Store methods that need at least this capability when a tool handler
/// calls them. The `_audited` form counts as the same call.
const PRIVILEGED_CALLS: &[(&str, &str)] = &[
    ("add_channel_member", capability::CHANNEL_ADMIN),
    ("allow_egress_target", capability::TOKEN_ADMIN),
    ("allow_secret_egress_host", capability::SECRET_ADMIN),
    ("catch_up_events", capability::TOKEN_ADMIN),
    ("clear_land_gate", capability::CHANNEL_ADMIN),
    ("clear_review_requirement", capability::CHANNEL_ADMIN),
    ("create_api_token", capability::TOKEN_ADMIN),
    ("create_attenuated_api_token", capability::TOKEN_ADMIN),
    ("create_delegated_api_token", capability::TOKEN_ADMIN),
    ("create_delegation_grant", capability::TOKEN_ADMIN),
    ("create_secret", capability::SECRET_ADMIN),
    ("create_share_ticket", capability::TOKEN_ADMIN),
    ("delete_secret", capability::SECRET_ADMIN),
    ("erase_workspace", capability::TOKEN_ADMIN),
    ("export_workspace", capability::TOKEN_ADMIN),
    ("freeze_member", capability::TOKEN_ADMIN),
    ("import_workspace", capability::TOKEN_ADMIN),
    ("list_channel_members", capability::CHANNEL_ADMIN),
    ("list_delegation_grants", capability::TOKEN_ADMIN),
    ("list_frozen_members", capability::TOKEN_ADMIN),
    ("list_secret_egress_hosts", capability::SECRET_ADMIN),
    ("list_share_tickets", capability::TOKEN_ADMIN),
    ("place_legal_hold", capability::TOKEN_ADMIN),
    ("purge_workspace_messages", capability::TOKEN_ADMIN),
    ("remove_channel_member", capability::CHANNEL_ADMIN),
    ("remove_reviewer", capability::CHANNEL_ADMIN),
    ("resolve_secret", capability::SECRET_READ),
    ("revoke_api_token", capability::TOKEN_ADMIN),
    ("revoke_delegation_grant", capability::TOKEN_ADMIN),
    ("revoke_egress_target", capability::TOKEN_ADMIN),
    ("revoke_secret_egress_host", capability::SECRET_ADMIN),
    ("revoke_share_ticket", capability::TOKEN_ADMIN),
    ("rotate_api_token", capability::TOKEN_ADMIN),
    ("set_delegation_policy", capability::TOKEN_ADMIN),
    ("set_retention_policy", capability::TOKEN_ADMIN),
    ("set_review_requirement", capability::THREAD_TRANSITION),
    ("unfreeze_member", capability::TOKEN_ADMIN),
];

/// Handler calls whose default minimum in [`PRIVILEGED_CALLS`] is broader
/// than the reviewed cap for this tool. The call is still part of the
/// tool's job: minting a subset of the holder, rotating only the bearer, or
/// seating the creator as admin of a private channel they just created.
const HANDLER_CALL_EXCEPTIONS: &[(&str, &str)] = &[
    ("attenuate_token", "create_api_token"),
    ("attenuate_token", "create_attenuated_api_token"),
    ("create_channel", "add_channel_member"),
    ("delegate_token", "create_delegated_api_token"),
    ("rotate_token", "rotate_api_token"),
];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("resolve repo root")
}

fn read(relative: &str) -> String {
    let path = repo_root().join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn tool_names() -> Vec<String> {
    serde_json::from_str(&read("contracts/mcp-tool-names.json")).expect("mcp-tool-names.json")
}

fn capability_map() -> BTreeMap<String, String> {
    serde_json::from_str(&read("contracts/mcp-capability-map.json"))
        .expect("mcp-capability-map.json")
}

fn is_read_cap(cap: &str) -> bool {
    READ_CAPS.contains(&cap)
}

fn pin_cap(rows: &[(&str, &str)], name: &str) -> Option<&'static str> {
    rows.iter()
        .find(|(tool, _)| *tool == name)
        .map(|(_, cap)| *cap)
}

fn declared_capability(name: &str) -> &'static str {
    required_capability(name).unwrap_or_else(|e| panic!("required_capability({name}): {e}"))
}

/// The `dispatch` and `dispatch_rest` bodies. Arm lookup has to start here,
/// not at the top of `tools/mod.rs`, because `required_capability` also has
/// `"tool" =>` arms and those would hide the handler.
fn dispatch_source(mod_src: &str) -> String {
    let mut out = String::new();
    for fn_name in ["dispatch", "dispatch_rest"] {
        let body = fn_body(mod_src, fn_name)
            .unwrap_or_else(|| panic!("tools/mod.rs should define {fn_name}"));
        out.push_str(body);
        out.push('\n');
    }
    out
}

fn dispatch_arms(mod_src: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let bytes = mod_src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let next = skip_non_code(bytes, i);
        if next != i {
            i = next;
            continue;
        }
        if bytes[i] == b'"' {
            if let Some((name, after)) = read_tool_arm(mod_src, i) {
                names.insert(name);
                i = after;
                continue;
            }
        }
        i += 1;
    }
    names
}

/// A `"tool_name" =>` arm starting at the opening quote, if that is one.
fn read_tool_arm(src: &str, quote_at: usize) -> Option<(String, usize)> {
    let rest = &src[quote_at + 1..];
    let end = rest.find('"')?;
    let name = &rest[..end];
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return None;
    }
    let after_name = quote_at + 1 + end + 1;
    let tail = src[after_name..].trim_start();
    if tail.starts_with("=>") {
        Some((name.to_string(), after_name))
    } else {
        None
    }
}

fn covers(declared: &str, needed: &str) -> bool {
    declared == needed
        || (declared == capability::SECRET_ADMIN && needed == capability::SECRET_READ)
}

fn is_exception(tool: &str, call: &str) -> bool {
    HANDLER_CALL_EXCEPTIONS
        .iter()
        .any(|(t, c)| *t == tool && *c == call)
}

/// Production source of every MCP tool module, tests blanked, keyed by
/// file stem (`channel`, `room`, `context`, …).
fn tool_modules(root: &Path) -> BTreeMap<String, String> {
    let mut modules = BTreeMap::new();
    let tools = root.join("crates/maidan-mcp/src/tools");
    for entry in std::fs::read_dir(&tools).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "rs") {
            let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
            let src = std::fs::read_to_string(&path).unwrap();
            modules.insert(stem, without_tests(&src));
        }
    }
    let context = root.join("crates/maidan-mcp/src/context.rs");
    if context.exists() {
        let src = std::fs::read_to_string(&context).unwrap();
        modules.insert("context".into(), without_tests(&src));
    }
    modules
}

/// Bodies a dispatch arm can reach: the arm itself, and one function it
/// calls in a tool module or in `context`.
fn handler_bodies(mod_src: &str, modules: &BTreeMap<String, String>, name: &str) -> String {
    let Some(arm) = arm_rhs(mod_src, name) else {
        return String::new();
    };
    let mut bodies = arm.clone();
    for (module, func) in callees(&arm) {
        if let Some(src) = modules.get(&module)
            && let Some(body) = fn_body(src, &func)
        {
            bodies.push('\n');
            bodies.push_str(body);
        }
    }
    bodies
}

fn arm_rhs(mod_src: &str, name: &str) -> Option<String> {
    let needle = format!("\"{name}\" =>");
    let at = mod_src.find(&needle)?;
    let start = at + needle.len();
    let first = start + mod_src[start..].find(|c: char| !c.is_whitespace())?;
    if mod_src.as_bytes()[first] == b'{' {
        let end = close_of(mod_src.as_bytes(), first);
        Some(mod_src[first..end].to_string())
    } else {
        let rel = mod_src[first..]
            .find([',', '\n'])
            .unwrap_or((mod_src.len() - first).min(400));
        Some(mod_src[first..first + rel].to_string())
    }
}

fn callees(src: &str) -> BTreeSet<(String, String)> {
    let mut out = BTreeSet::new();
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let next = skip_non_code(bytes, i);
        if next != i {
            i = next;
            continue;
        }
        if ident_start(bytes[i]) {
            let (ident, after) = read_ident(src, i);
            if ident == "crate" && src[after..].starts_with("::") {
                let mut j = after + 2;
                let mut parts = Vec::new();
                loop {
                    if j >= src.len() || !ident_start(bytes[j]) {
                        break;
                    }
                    let (part, next) = read_ident(src, j);
                    parts.push(part);
                    j = next;
                    if src[j..].starts_with("::") {
                        j += 2;
                    } else {
                        break;
                    }
                }
                if src[j..].trim_start().starts_with('(') && parts.len() >= 2 {
                    let func = parts.last().unwrap().clone();
                    let module = parts[parts.len() - 2].clone();
                    out.insert((module, func));
                }
                i = j;
                continue;
            }
            if src[after..].starts_with("::") {
                let (func, after_fn) = read_ident(src, after + 2);
                if src[after_fn..].trim_start().starts_with('(') && !func.is_empty() {
                    out.insert((ident, func));
                }
                i = after_fn;
                continue;
            }
            i = after;
            continue;
        }
        i += 1;
    }
    out
}

fn fn_body<'a>(src: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!("fn {name}(");
    let mut from = 0;
    while let Some(rel) = src[from..].find(&needle) {
        let at = from + rel;
        if at > 0 {
            let prev = src.as_bytes()[at - 1];
            if prev.is_ascii_alphanumeric() || prev == b'_' {
                from = at + 1;
                continue;
            }
        }
        let after_sig = at + needle.len();
        let brace = src[after_sig..].find('{')?;
        let open = after_sig + brace;
        let end = close_of(src.as_bytes(), open);
        return Some(&src[open..end]);
    }
    None
}

fn privileged_calls_in(body: &str) -> BTreeSet<&'static str> {
    let mut found = BTreeSet::new();
    for line in body.lines() {
        if line.trim_start().starts_with("//") {
            continue;
        }
        for (call, _) in PRIVILEGED_CALLS {
            if line.contains(&format!(".{call}(")) || line.contains(&format!(".{call}_audited(")) {
                found.insert(*call);
            }
        }
    }
    found
}

fn ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn read_ident(src: &str, i: usize) -> (String, usize) {
    let bytes = src.as_bytes();
    let mut j = i;
    while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
        j += 1;
    }
    (src[i..j].to_string(), j)
}

/// Index just past a comment or literal starting at `i`, or `i` when the
/// byte is code. Enough to skip `{` and `=>` inside strings and comments.
fn skip_non_code(src: &[u8], i: usize) -> usize {
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
        let mut j = i + 2;
        while j + 1 < src.len() && !(src[j] == b'*' && src[j + 1] == b'/') {
            j += 1;
        }
        return (j + 2).min(src.len());
    }
    if src[i] == b'"' {
        return skip_cooked(src, i);
    }
    if src[i] == b'r' {
        let mut k = i + 1;
        let mut hashes = 0usize;
        while src.get(k) == Some(&b'#') {
            hashes += 1;
            k += 1;
        }
        if src.get(k) == Some(&b'"') {
            let closer = {
                let mut c = vec![b'"'];
                c.extend(std::iter::repeat_n(b'#', hashes));
                c
            };
            let mut j = k + 1;
            while j < src.len() && !src[j..].starts_with(&closer) {
                j += 1;
            }
            return if j >= src.len() {
                src.len()
            } else {
                j + closer.len()
            };
        }
    }
    i
}

fn skip_cooked(src: &[u8], quote_at: usize) -> usize {
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

/// The index just past the bracket group that opens at `open`.
fn close_of(src: &[u8], open: usize) -> usize {
    let mut depth = 0usize;
    let mut i = open;
    while i < src.len() {
        let next = skip_non_code(src, i);
        if next != i {
            i = next;
            continue;
        }
        match src[i] {
            b'{' | b'(' | b'[' => depth += 1,
            b'}' | b')' | b']' => {
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

#[test]
fn every_mcp_tool_declares_a_known_capability() {
    let names = tool_names();
    let map = capability_map();
    let dispatch = dispatch_source(&without_tests(&read("crates/maidan-mcp/src/tools/mod.rs")));
    let arms = dispatch_arms(&dispatch);
    assert!(
        names.len() > 50,
        "expected a full tool catalog, found {}",
        names.len()
    );

    let mut missing = Vec::new();
    for name in &names {
        let cap = match required_capability(name) {
            Ok(cap) => cap,
            Err(err) => {
                missing.push(format!("{name}: required_capability: {err}"));
                continue;
            }
        };
        if !is_known(cap) {
            missing.push(format!("{name}: unknown capability {cap}"));
        }
        match map.get(name).map(String::as_str) {
            Some(mapped) if mapped == cap => {}
            Some(mapped) => missing.push(format!(
                "{name}: map has {mapped}, required_capability returns {cap}"
            )),
            None => missing.push(format!("{name}: missing from mcp-capability-map.json")),
        }
        if !arms.contains(name) {
            missing.push(format!("{name}: no dispatch arm in tools/mod.rs"));
        }
    }
    for key in map.keys() {
        if !names.iter().any(|n| n == key) {
            missing.push(format!(
                "{key}: in mcp-capability-map.json but not mcp-tool-names.json"
            ));
        }
    }
    assert!(
        missing.is_empty(),
        "every MCP tool must declare a known capability and have a dispatch arm:\n  {}",
        missing.join("\n  ")
    );
}

#[test]
fn a_read_only_tool_declares_a_read_capability_or_is_a_named_privileged_read() {
    let mut unexpected = Vec::new();
    for name in tool_names() {
        let cap = declared_capability(&name);
        let pinned = pin_cap(PRIVILEGED_READS, &name);
        if is_read_only(&name) {
            if let Some(expected) = pinned {
                if cap != expected {
                    unexpected.push(format!(
                        "{name}: privileged read pin is {expected}, declaration is {cap}"
                    ));
                }
            } else if !is_read_cap(cap) {
                unexpected.push(format!(
                    "{name}: read-only but declares {cap}; add a PRIVILEGED_READS pin if that \
                     is the narrowest cap that covers the data"
                ));
            }
        } else if pinned.is_some() {
            unexpected.push(format!(
                "{name}: pinned as a privileged read but is not in READ_ONLY_TOOLS"
            ));
        }
    }
    assert!(
        unexpected.is_empty(),
        "read-only tools must declare a read capability, or be named:\n  {}",
        unexpected.join("\n  ")
    );
}

#[test]
fn a_mutating_tool_declares_a_write_capability_or_is_a_named_read_cap_write() {
    let mut unexpected = Vec::new();
    for name in tool_names() {
        let cap = declared_capability(&name);
        let pinned = pin_cap(READ_CAP_WRITES, &name);
        if !is_read_only(&name) {
            if let Some(expected) = pinned {
                if cap != expected {
                    unexpected.push(format!(
                        "{name}: read-cap write pin is {expected}, declaration is {cap}"
                    ));
                }
            } else if is_read_cap(cap) {
                unexpected.push(format!(
                    "{name}: mutates but declares {cap}; add a READ_CAP_WRITES pin if this is \
                     personal or holder-side state"
                ));
            }
        } else if pinned.is_some() {
            unexpected.push(format!(
                "{name}: pinned as a read-cap write but is in READ_ONLY_TOOLS"
            ));
        }
    }
    assert!(
        unexpected.is_empty(),
        "mutating tools must declare a write or authority capability, or be named:\n  {}",
        unexpected.join("\n  ")
    );
}

#[test]
fn a_handler_does_not_exceed_its_declared_capability() {
    let root = repo_root();
    let dispatch = dispatch_source(&without_tests(&read("crates/maidan-mcp/src/tools/mod.rs")));
    let modules = tool_modules(&root);
    let mut offenders = Vec::new();
    let mut seen_exceptions: BTreeSet<(String, &str)> = BTreeSet::new();

    for name in tool_names() {
        let declared = declared_capability(&name);
        let body = handler_bodies(&dispatch, &modules, &name);
        assert!(
            !body.is_empty(),
            "{name}: could not find a dispatch arm or handler body"
        );
        for call in privileged_calls_in(&body) {
            let needed = PRIVILEGED_CALLS
                .iter()
                .find(|(c, _)| *c == call)
                .map(|(_, cap)| *cap)
                .expect("call came from PRIVILEGED_CALLS");
            if is_exception(&name, call) {
                seen_exceptions.insert((name.clone(), call));
                continue;
            }
            if !covers(declared, needed) {
                offenders.push(format!(
                    "{name}: handler calls {call} (needs {needed}) but declares {declared}"
                ));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "a tool handler grants more than its declaration:\n  {}",
        offenders.join("\n  ")
    );
    for (tool, call) in HANDLER_CALL_EXCEPTIONS {
        assert!(
            seen_exceptions.iter().any(|(t, c)| t == tool && *c == call),
            "{tool} no longer calls {call}; update HANDLER_CALL_EXCEPTIONS"
        );
    }
}

#[test]
fn named_exceptions_still_match_the_catalog() {
    let names: BTreeSet<String> = tool_names().into_iter().collect();
    let mut missing = Vec::new();
    for tool in PRIVILEGED_READS
        .iter()
        .map(|(tool, _)| *tool)
        .chain(READ_CAP_WRITES.iter().map(|(tool, _)| *tool))
        .chain(HANDLER_CALL_EXCEPTIONS.iter().map(|(tool, _)| *tool))
    {
        if !names.contains(tool) {
            missing.push(tool);
        }
    }
    assert!(
        missing.is_empty(),
        "pinned tools must still be in the catalog: {missing:?}"
    );
}
