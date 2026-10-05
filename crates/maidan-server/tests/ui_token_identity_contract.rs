//! Pasting a token must not keep the previous member when `/me` never answers.
//! The page handler is the one in `static/ui/session.js`. Node runs that
//! module against a stub document. A copy of the handler would not count.

use std::process::Command;

#[test]
fn a_thrown_me_check_drops_the_previous_bearer_member() {
    let source = include_str!("../static/ui/session.js");
    let handler = source
        .split("async function trySignIn()")
        .nth(1)
        .expect("trySignIn function");
    let catch_block = handler
        .split("} catch (e) {")
        .nth(1)
        .expect("me failure catch")
        .split("if (!res.ok)")
        .next()
        .expect("catch ends before the status check");
    assert!(
        catch_block.contains("forgetBearerMember();"),
        "a thrown /me check has to drop the member learned from the previous token"
    );
    assert!(
        catch_block.find("forgetBearerMember();").unwrap() < catch_block.find("return;").unwrap(),
        "the member is dropped before the handler returns"
    );

    let harness = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/ui_token_identity_harness.mjs"
    );
    let output = Command::new("node")
        .arg(harness)
        .output()
        .expect("node is required to run the token identity harness");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "token identity harness failed\n{stderr}\n{stdout}"
    );
}
