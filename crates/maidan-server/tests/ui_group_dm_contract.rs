//! Opening a group DM matches the store and the one-to-one DM path.
//!
//! The store refuses a conversation with fewer than three members. The page
//! says that before it posts, and a successful open selects the conversation
//! the response returned, the same way opening a DM selects that DM.

const DM_JS: &str = include_str!("../static/ui/dm.js");
const HTML: &str = include_str!("../static/index.html");

fn open_group_dm() -> &'static str {
    let start = DM_JS
        .find("async function openGroupDm()")
        .expect("openGroupDm");
    let end = DM_JS[start..]
        .find("async function sendGroupDmMessage()")
        .expect("sendGroupDmMessage");
    &DM_JS[start..start + end]
}

#[test]
fn opening_a_group_dm_requires_three_members_and_selects_it() {
    let body = open_group_dm();
    assert!(
        body.contains(
            "if (ids.length < 3) return showError(\"A group DM needs at least 3 members\")"
        ),
        "the page refuses fewer than three members, which is what the store requires"
    );
    assert!(
        !body.contains("at least 2 members"),
        "two members is not a group DM"
    );
    assert!(
        body.contains("showError(await responseError(res, \"Could not open that group DM\"))"),
        "a refused open is a sentence, not a raw status"
    );
    assert!(
        !body.contains("HTTP ${res.status}"),
        "openGroupDm does not paint a raw status"
    );
    assert!(
        body.contains("const opened = await res.json();")
            && body.contains("await loadMembers();")
            && body.contains("selectGroupDm(opened);"),
        "a successful open selects the conversation the server returned"
    );
    assert!(
        HTML.contains("at least two others"),
        "the form says the signed-in person plus two others"
    );
}
