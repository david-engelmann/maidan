//! The SQL that limits a lookup to the workspaces an inbound route serves
//! ([`InboundScope`]). Both backends build their queries with it so the rule
//! reads the same in each: a workspace's own route reaches that workspace, and
//! the server's route reaches the workspaces the operator opened that stored no
//! connection of their own on the surface.

use maidan_types::InboundScope;

/// A condition on `workspace_column` for `scope`. `placeholder` is the bind
/// the caller supplies next, `?` or `$n`: the workspace id for a workspace's
/// route, the surface's reserved secret name for the server's.
pub(crate) fn clause(scope: InboundScope, workspace_column: &str, placeholder: &str) -> String {
    match scope {
        InboundScope::Workspace(_) => format!("{workspace_column} = {placeholder}"),
        InboundScope::Instance(_) => format!(
            "EXISTS (SELECT 1 FROM maidan_workspaces w
                     WHERE w.id = {workspace_column} AND w.origin = 'operator'
                       AND NOT EXISTS (SELECT 1 FROM maidan_secrets s
                                       WHERE s.workspace_id = w.id AND s.name = {placeholder}))"
        ),
    }
}

/// What to bind for [`clause`].
pub(crate) enum Bind {
    Workspace(uuid::Uuid),
    Secret(&'static str),
}

pub(crate) fn bind(scope: InboundScope) -> Bind {
    match scope {
        InboundScope::Workspace(workspace_id) => Bind::Workspace(workspace_id.0),
        InboundScope::Instance(surface) => Bind::Secret(surface.secret_name()),
    }
}
