//! SCIM 2.0 Groups (RFC 7643 §4.2, RFC 7644 §3): list (with `displayName`,
//! `externalId` or `id eq` filters, `startIndex`/`count` paging and
//! `excludedAttributes=members`), get, create, replace, patch and delete.
//!
//! A group is the IdP's named set of users it provisioned into the token's
//! workspace; it grants nothing by itself. Its members are SCIM users of that
//! workspace only: an id that is another workspace's member, or a member the
//! IdP did not provision, is refused with the same `invalidValue` error as an
//! id that does not exist, so the answer says nothing about other tenants.
//!
//! PATCH takes both shapes the large IdPs send. Okta removes a member with a
//! filter path (`"path": "members[value eq \"<id>\"]"`) and renames with a
//! pathless `replace` whose value is an object; Entra ID removes with
//! `"path": "members"` and a value array, and capitalizes `op` (`"Add"`,
//! `"Remove"`, `"Replace"`). Every write commits with its audit row (D-A).

use axum::extract::{RawQuery, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Extension;
use maidan_auth::AuthContext;
use maidan_store::StoreError;
use maidan_types::{
    MemberId, NewAuditEvent, NewScimGroup, ScimGroup, ScimGroupChange, ScimGroupId, ScimMembersOp,
};
use serde_json::{json, Value};

use crate::scim::{
    eq_filter, parse_eq_filter, query_param, require_admin, scim_error, scim_error_typed,
    scim_response, ScimFault, ScimPatchOp, ScimPath, ScimText, LIST_SCHEMA,
};
use crate::state::AppState;

const GROUP_SCHEMA: &str = "urn:ietf:params:scim:schemas:core:2.0:Group";
/// The largest page a list returns, as `ServiceProviderConfig` advertises.
const MAX_RESULTS: usize = 200;

fn invalid_value(detail: &str) -> ScimFault {
    ScimFault::invalid_value(detail)
}

fn no_such_group() -> Response {
    scim_error(StatusCode::NOT_FOUND, "no such group")
}

fn store_failure(err: StoreError) -> Response {
    match err {
        StoreError::InvalidInput(detail) => invalid_value(&detail).into_response(),
        err => scim_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("group write failed: {err}"),
        ),
    }
}

/// Render a group as a SCIM Group resource. `with_members` is false when the
/// caller asked for `excludedAttributes=members`.
fn group_resource(group: &ScimGroup, with_members: bool) -> Value {
    let mut resource = json!({
        "schemas": [GROUP_SCHEMA],
        "id": group.id.0.to_string(),
        "displayName": group.display_name,
        "meta": {
            "resourceType": "Group",
            "created": group.created_at.to_rfc3339(),
            "lastModified": group.updated_at.to_rfc3339(),
            "location": format!("/scim/v2/Groups/{}", group.id.0),
        }
    });
    if let Some(ext) = &group.external_id {
        resource["externalId"] = json!(ext);
    }
    if with_members {
        resource["members"] = group
            .members
            .iter()
            .map(|m| {
                json!({
                    "value": m.member_id.0.to_string(),
                    "display": m.handle,
                    "type": "User",
                    "$ref": format!("/scim/v2/Users/{}", m.member_id.0),
                })
            })
            .collect();
    }
    resource
}

fn members_excluded(raw_query: Option<&str>) -> bool {
    raw_query
        .and_then(|q| query_param(q, "excludedAttributes"))
        .is_some_and(|list| {
            list.split(',')
                .any(|attr| attr.trim().eq_ignore_ascii_case("members"))
        })
}

/// Member ids from a SCIM `members` value: an array of `{"value": "<id>"}`,
/// or one such object.
fn member_ids(value: &Value) -> Result<Vec<MemberId>, ScimFault> {
    let items: Vec<&Value> = match value {
        Value::Array(items) => items.iter().collect(),
        Value::Object(_) => vec![value],
        Value::Null => vec![],
        _ => return Err(invalid_value("members must be an array of {\"value\": id}")),
    };
    items
        .into_iter()
        .map(|item| {
            item.get("value")
                .and_then(Value::as_str)
                .and_then(|id| uuid::Uuid::parse_str(id.trim()).ok())
                .map(MemberId)
                .ok_or_else(|| invalid_value("a member's value must be a user id"))
        })
        .collect()
}

fn display_name_from(value: &Value) -> Result<String, ScimFault> {
    value
        .as_str()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .ok_or_else(|| invalid_value("displayName must be a non-empty string"))
}

fn external_id_from(value: &Value) -> Result<Option<String>, ScimFault> {
    match value {
        Value::Null => Ok(None),
        Value::String(s) => Ok(Some(s.clone())),
        _ => Err(invalid_value("externalId must be a string")),
    }
}

/// A group body for POST and PUT: `displayName` is required, and `externalId`
/// and `members` absent mean none.
fn group_body(body: &str) -> Result<(String, Option<String>, Vec<MemberId>), ScimFault> {
    let value: Value = serde_json::from_str(body).map_err(|e| {
        ScimFault::new(
            StatusCode::BAD_REQUEST,
            "invalidSyntax",
            &format!("invalid SCIM Group: {e}"),
        )
    })?;
    let display_name = display_name_from(value.get("displayName").unwrap_or(&Value::Null))?;
    let external_id = external_id_from(value.get("externalId").unwrap_or(&Value::Null))?;
    let members = member_ids(value.get("members").unwrap_or(&Value::Null))?;
    Ok((display_name, external_id, members))
}

/// `GET /scim/v2/Groups`.
pub async fn list_groups(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    RawQuery(query): RawQuery,
) -> Response {
    if let Some(resp) = require_admin(&auth) {
        return resp;
    }
    let raw = query.as_deref().unwrap_or_default();
    let filter = match eq_filter(
        raw,
        &["displayname", "externalid", "id"],
        "Groups supports displayName, externalId and id with eq",
    ) {
        Ok(filter) => filter,
        Err(fault) => return fault.into_response(),
    };
    let groups = match state.store.list_scim_groups(auth.workspace_id).await {
        Ok(groups) => groups,
        Err(err) => return store_failure(err),
    };
    let matching: Vec<&ScimGroup> = groups
        .iter()
        .filter(|g| match &filter {
            None => true,
            // displayName is not caseExact (RFC 7643 §4.2); externalId is.
            Some((attr, value)) if attr == "displayname" => {
                g.display_name.eq_ignore_ascii_case(value)
            }
            Some((attr, value)) if attr == "externalid" => {
                g.external_id.as_deref() == Some(value.as_str())
            }
            Some((_, value)) => g.id.0.to_string().eq_ignore_ascii_case(value),
        })
        .collect();
    // startIndex is 1-based; values below 1 mean 1 (RFC 7644 §3.4.2.4).
    let start = query_param(raw, "startIndex")
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(1)
        .max(1);
    let count = query_param(raw, "count")
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(MAX_RESULTS)
        .min(MAX_RESULTS);
    let with_members = !members_excluded(query.as_deref());
    let page: Vec<Value> = matching
        .iter()
        .skip(start - 1)
        .take(count)
        .map(|g| group_resource(g, with_members))
        .collect();
    scim_response(
        StatusCode::OK,
        json!({
            "schemas": [LIST_SCHEMA],
            "totalResults": matching.len(),
            "startIndex": start,
            "itemsPerPage": page.len(),
            "Resources": page,
        }),
    )
}

/// `GET /scim/v2/Groups/:id`.
pub async fn get_group(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ScimPath(id): ScimPath<uuid::Uuid>,
    RawQuery(query): RawQuery,
) -> Response {
    if let Some(resp) = require_admin(&auth) {
        return resp;
    }
    match state
        .store
        .get_scim_group(auth.workspace_id, ScimGroupId(id))
        .await
    {
        Ok(Some(group)) => scim_response(
            StatusCode::OK,
            group_resource(&group, !members_excluded(query.as_deref())),
        ),
        Ok(None) => no_such_group(),
        Err(err) => store_failure(err),
    }
}

/// `POST /scim/v2/Groups`.
pub async fn create_group(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ScimText(body): ScimText,
) -> Response {
    if let Some(resp) = require_admin(&auth) {
        return resp;
    }
    let (display_name, external_id, members) = match group_body(&body) {
        Ok(parsed) => parsed,
        Err(fault) => return fault.into_response(),
    };
    let actor = auth.actor_id;
    match state
        .store
        .scim_create_group_audited(
            NewScimGroup {
                workspace_id: auth.workspace_id,
                display_name,
                external_id,
                members,
            },
            Box::new(move |group| NewAuditEvent {
                actor_id: Some(actor),
                action: "scim.group.create".into(),
                target_kind: Some("scim_group".into()),
                target_id: Some(group.id.0),
                metadata: json!({
                    "displayName": group.display_name,
                    "members": group.members.iter().map(|m| m.member_id.0).collect::<Vec<_>>(),
                }),
            }),
        )
        .await
    {
        Ok(group) => scim_response(StatusCode::CREATED, group_resource(&group, true)),
        Err(err) => store_failure(err),
    }
}

/// Apply a change to a group and answer with the group as it now is.
async fn update_group(
    state: &AppState,
    auth: &AuthContext,
    id: ScimGroupId,
    change: ScimGroupChange,
) -> Response {
    let actor = auth.actor_id;
    match state
        .store
        .scim_update_group_audited(
            auth.workspace_id,
            id,
            change,
            Box::new(move |write| NewAuditEvent {
                actor_id: Some(actor),
                action: "scim.group.update".into(),
                target_kind: Some("scim_group".into()),
                target_id: Some(write.group.id.0),
                metadata: json!({
                    "displayName": write.group.display_name,
                    "added": write.added.iter().map(|m| m.0).collect::<Vec<_>>(),
                    "removed": write.removed.iter().map(|m| m.0).collect::<Vec<_>>(),
                }),
            }),
        )
        .await
    {
        Ok(Some(write)) => scim_response(StatusCode::OK, group_resource(&write.group, true)),
        Ok(None) => no_such_group(),
        Err(err) => store_failure(err),
    }
}

/// `PUT /scim/v2/Groups/:id` — replace: the body is the whole group, so an
/// absent `members` leaves the group empty (RFC 7644 §3.5.1).
pub async fn replace_group(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ScimPath(id): ScimPath<uuid::Uuid>,
    ScimText(body): ScimText,
) -> Response {
    if let Some(resp) = require_admin(&auth) {
        return resp;
    }
    let (display_name, external_id, members) = match group_body(&body) {
        Ok(parsed) => parsed,
        Err(fault) => return fault.into_response(),
    };
    let change = ScimGroupChange {
        display_name: Some(display_name),
        external_id: Some(external_id),
        members: vec![ScimMembersOp::Replace(members)],
    };
    update_group(&state, &auth, ScimGroupId(id), change).await
}

/// The member id a filter path names: `members[value eq "<id>"]`.
fn filtered_member(path: &str) -> Option<Result<MemberId, ScimFault>> {
    let lower = path.to_ascii_lowercase();
    if !lower.starts_with("members[") || !lower.ends_with(']') {
        return None;
    }
    let inner = &path["members[".len()..path.len() - 1];
    Some(
        parse_eq_filter(inner)
            .filter(|(attr, _)| attr == "value")
            .and_then(|(_, id)| uuid::Uuid::parse_str(&id).ok())
            .map(MemberId)
            .ok_or_else(|| {
                ScimFault::new(
                    StatusCode::BAD_REQUEST,
                    "invalidPath",
                    "a members filter must be members[value eq \"<id>\"]",
                )
            }),
    )
}

/// Turn a PatchOp into a group change, operations in order.
fn patch_change(patch: &ScimPatchOp) -> Result<ScimGroupChange, ScimFault> {
    let mut change = ScimGroupChange::default();
    for op in &patch.operations {
        let kind = op.op.to_ascii_lowercase();
        if !["add", "remove", "replace"].contains(&kind.as_str()) {
            return Err(ScimFault::new(
                StatusCode::BAD_REQUEST,
                "invalidSyntax",
                &format!("unsupported PatchOp op {:?}", op.op),
            ));
        }
        let Some(path) = op.path.as_deref().map(str::trim) else {
            if kind == "remove" {
                return Err(ScimFault::new(
                    StatusCode::BAD_REQUEST,
                    "noTarget",
                    "remove needs a path",
                ));
            }
            // A pathless add or replace carries an object of attributes.
            let Some(attrs) = op.value.as_object() else {
                return Err(invalid_value(
                    "a pathless operation's value must be an object",
                ));
            };
            for (attr, value) in attrs {
                if attr.eq_ignore_ascii_case("displayName") {
                    change.display_name = Some(display_name_from(value)?);
                } else if attr.eq_ignore_ascii_case("externalId") {
                    change.external_id = Some(external_id_from(value)?);
                } else if attr.eq_ignore_ascii_case("members") {
                    let ids = member_ids(value)?;
                    change.members.push(if kind == "add" {
                        ScimMembersOp::Add(ids)
                    } else {
                        ScimMembersOp::Replace(ids)
                    });
                }
                // Okta repeats the group's `id` here; it and other
                // attributes are ignored, as the user endpoint ignores them.
            }
            continue;
        };
        if let Some(member) = filtered_member(path) {
            let member = member?;
            if kind != "remove" {
                return Err(ScimFault::new(
                    StatusCode::BAD_REQUEST,
                    "invalidPath",
                    "a members filter path is supported for remove only",
                ));
            }
            change.members.push(ScimMembersOp::Remove(vec![member]));
        } else if path.eq_ignore_ascii_case("members") {
            change.members.push(match kind.as_str() {
                "add" => ScimMembersOp::Add(member_ids(&op.value)?),
                "replace" => ScimMembersOp::Replace(member_ids(&op.value)?),
                // With no value, remove every member (RFC 7644 §3.5.2.2).
                _ if op.value.is_null() => ScimMembersOp::Replace(vec![]),
                _ => ScimMembersOp::Remove(member_ids(&op.value)?),
            });
        } else if path.eq_ignore_ascii_case("displayName") {
            if kind == "remove" {
                return Err(ScimFault::new(
                    StatusCode::BAD_REQUEST,
                    "mutability",
                    "displayName is required",
                ));
            }
            change.display_name = Some(display_name_from(&op.value)?);
        } else if path.eq_ignore_ascii_case("externalId") {
            change.external_id = Some(if kind == "remove" {
                None
            } else {
                external_id_from(&op.value)?
            });
        }
    }
    Ok(change)
}

/// `PATCH /scim/v2/Groups/:id` — `add`, `remove` and `replace` of `members`,
/// and `replace` of `displayName` and `externalId`, in Okta's and Entra ID's
/// shapes (module doc).
pub async fn patch_group(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ScimPath(id): ScimPath<uuid::Uuid>,
    ScimText(body): ScimText,
) -> Response {
    if let Some(resp) = require_admin(&auth) {
        return resp;
    }
    let patch: ScimPatchOp = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => {
            return scim_error_typed(
                StatusCode::BAD_REQUEST,
                "invalidSyntax",
                &format!("invalid PatchOp: {e}"),
            )
        }
    };
    let change = match patch_change(&patch) {
        Ok(change) => change,
        Err(fault) => return fault.into_response(),
    };
    update_group(&state, &auth, ScimGroupId(id), change).await
}

/// `DELETE /scim/v2/Groups/:id` — the group and its memberships; the members
/// themselves are untouched.
pub async fn delete_group(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ScimPath(id): ScimPath<uuid::Uuid>,
) -> Response {
    if let Some(resp) = require_admin(&auth) {
        return resp;
    }
    let id = ScimGroupId(id);
    let group = match state.store.get_scim_group(auth.workspace_id, id).await {
        Ok(Some(group)) => group,
        Ok(None) => return no_such_group(),
        Err(err) => return store_failure(err),
    };
    let event = NewAuditEvent {
        actor_id: Some(auth.actor_id),
        action: "scim.group.delete".into(),
        target_kind: Some("scim_group".into()),
        target_id: Some(id.0),
        metadata: json!({
            "displayName": group.display_name,
            "members": group.members.len(),
        }),
    };
    match state
        .store
        .scim_delete_group_audited(auth.workspace_id, id, event)
        .await
    {
        Ok(true) => (StatusCode::NO_CONTENT, ()).into_response(),
        Ok(false) => no_such_group(),
        Err(err) => store_failure(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(ops: Value) -> ScimPatchOp {
        serde_json::from_value(json!({ "schemas": [crate::scim::PATCH_SCHEMA], "Operations": ops }))
            .unwrap()
    }

    const A: &str = "0192f0c1-0000-7000-8000-000000000001";
    const B: &str = "0192f0c1-0000-7000-8000-000000000002";

    fn id(s: &str) -> MemberId {
        MemberId(uuid::Uuid::parse_str(s).unwrap())
    }

    #[test]
    fn okta_patch_shapes_become_group_changes() {
        // Okta: add with a members path, remove with a filter path, rename
        // with a pathless replace that repeats the group id.
        let change = patch_change(&patch(json!([
            { "op": "add", "path": "members", "value": [{ "value": A, "display": "a@x" }] },
            { "op": "remove", "path": format!("members[value eq \"{B}\"]") },
            { "op": "replace", "value": { "id": "ignored", "displayName": "Eng" } }
        ])))
        .unwrap();
        assert_eq!(
            change.members,
            vec![
                ScimMembersOp::Add(vec![id(A)]),
                ScimMembersOp::Remove(vec![id(B)])
            ]
        );
        assert_eq!(change.display_name.as_deref(), Some("Eng"));
    }

    #[test]
    fn entra_patch_shapes_become_group_changes() {
        // Entra ID: capitalized ops, remove with a members path and a value
        // array, and `$ref: null` on each member.
        let change = patch_change(&patch(json!([
            { "op": "Add", "path": "members", "value": [{ "$ref": null, "value": A }] },
            { "op": "Remove", "path": "members", "value": [{ "$ref": null, "value": B }] },
            { "op": "Replace", "path": "displayName", "value": "Eng 2" },
            { "op": "Replace", "path": "externalId", "value": "entra-9" }
        ])))
        .unwrap();
        assert_eq!(
            change.members,
            vec![
                ScimMembersOp::Add(vec![id(A)]),
                ScimMembersOp::Remove(vec![id(B)])
            ]
        );
        assert_eq!(change.display_name.as_deref(), Some("Eng 2"));
        assert_eq!(change.external_id, Some(Some("entra-9".into())));
    }

    #[test]
    fn remove_of_members_without_a_value_removes_everyone() {
        let change = patch_change(&patch(json!([{ "op": "remove", "path": "members" }]))).unwrap();
        assert_eq!(change.members, vec![ScimMembersOp::Replace(vec![])]);
    }

    #[test]
    fn malformed_operations_are_refused() {
        for ops in [
            json!([{ "op": "move", "path": "members", "value": [] }]),
            json!([{ "op": "remove" }]),
            json!([{ "op": "add", "path": "members", "value": [{ "value": "not-a-uuid" }] }]),
            json!([{ "op": "add", "path": format!("members[value eq \"{A}\"]") }]),
            json!([{ "op": "replace", "path": "displayName", "value": "  " }]),
        ] {
            assert!(patch_change(&patch(ops.clone())).is_err(), "{ops}");
        }
    }
}
