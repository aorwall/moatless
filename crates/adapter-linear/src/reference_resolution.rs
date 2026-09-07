use super::agent_api::{LinearAgentApi, LookupKind};
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_ports::identity::UserDisplay;
use serde_json::{json, Value};
use tracing::warn;

/// Heuristic: a Linear UUID is 36 chars with dashes at positions 8/13/18/23.
/// Identifiers like `SP-123` start with letters and a dash. Anything that
/// parses as a UUID is treated as already-resolved.
pub(super) fn looks_like_uuid(s: &str) -> bool {
    uuid::Uuid::parse_str(s).is_ok()
}

/// Resolve a value that may be either a UUID (passthrough) or a name
/// (lookup). Returns `Ok(None)` for `None` input.
pub(super) async fn resolve_optional(
    api: &LinearAgentApi,
    kind: LookupKind,
    value: Option<&str>,
) -> Result<Option<String>, AdapterError> {
    let Some(value) = value else { return Ok(None) };
    if looks_like_uuid(value) {
        return Ok(Some(value.to_string()));
    }
    let lookup = api.lookup(kind, value).await?.ok_or_else(|| {
        SharedAdapterError::NotFound(format!("Linear {kind:?} `{value}` not found"))
    })?;
    Ok(Some(lookup.id))
}

/// Required variant of [`resolve_optional`].
pub(super) async fn resolve_required(
    api: &LinearAgentApi,
    kind: LookupKind,
    value: &str,
) -> Result<String, AdapterError> {
    resolve_optional(api, kind, Some(value))
        .await?
        .ok_or_else(|| {
            SharedAdapterError::Internal("resolve_optional returned None unexpectedly".into())
                .into()
        })
}

/// Resolve a team value (key like `SP`, or UUID). Lookup uses the team's
/// `name` field, so a team key like `SP` is checked via a key filter first.
pub(super) async fn resolve_team_id(
    api: &LinearAgentApi,
    value: &str,
) -> Result<String, AdapterError> {
    if looks_like_uuid(value) {
        return Ok(value.to_string());
    }
    // Try by key first (e.g. `SP`), then fall back to name lookup.
    let by_key = api
        .graphql(json!({
            "query": "query($key: String!) { teams(filter: { key: { eqIgnoreCase: $key } }, first: 1) { nodes { id key name } } }",
            "variables": { "key": value }
        }))
        .await?;
    if let Some(id) = by_key
        .pointer("/data/teams/nodes/0/id")
        .and_then(Value::as_str)
    {
        return Ok(id.to_string());
    }
    let lookup = api
        .lookup(LookupKind::Team, value)
        .await?
        .ok_or_else(|| SharedAdapterError::NotFound(format!("Linear team `{value}` not found")))?;
    Ok(lookup.id)
}

/// Return the default team id when the caller didn't specify one. If
/// exactly one team is accessible we use it; otherwise surface the list
/// and require a `--team` flag.
pub(super) async fn default_team_id(api: &LinearAgentApi) -> Result<String, AdapterError> {
    let value = api
        .graphql(json!({
            "query": "query { teams(first: 5) { nodes { id key name } } }"
        }))
        .await?;
    let nodes = value
        .pointer("/data/teams/nodes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    match nodes.len() {
        0 => Err(
            SharedAdapterError::BadRequest("No Linear teams accessible — pass `team`".into())
                .into(),
        ),
        1 => nodes[0]
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| SharedAdapterError::AdapterApi("Linear team missing id".into()).into()),
        _ => {
            let keys: Vec<String> = nodes
                .iter()
                .filter_map(|n| n.get("key").and_then(Value::as_str).map(str::to_string))
                .collect();
            Err(SharedAdapterError::BadRequest(format!(
                "Multiple Linear teams accessible: [{}]. Pass `team` to choose.",
                keys.join(", ")
            ))
            .into())
        }
    }
}

/// A resolved Linear user with the fields needed to wire it into an issue as
/// assignee (`id`) or reporter (`id` for the subscriber, `url` for the
/// description mention).
pub(super) struct ResolvedUser {
    pub(super) id: String,
    pub(super) url: String,
}

/// Build a [`ResolvedUser`] from a GraphQL user node selecting `id url`.
pub(super) fn user_from_node(node: &Value) -> Option<ResolvedUser> {
    let id = node.get("id").and_then(Value::as_str)?.to_string();
    let url = node
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Some(ResolvedUser { id, url })
}

/// Resolve a user reference (`me`, UUID, email, or display name) to a Linear
/// user. Returns `Ok(None)` when no user matches — callers decide whether a
/// miss is fatal (explicit `--assignee`/`--reporter`) or skippable (the
/// best-effort assignee default).
pub(super) async fn resolve_user_opt(
    api: &LinearAgentApi,
    value: &str,
) -> Result<Option<ResolvedUser>, AdapterError> {
    if value == "me" {
        let viewer = api
            .graphql(json!({ "query": "query { viewer { id url } }" }))
            .await?;
        return Ok(viewer.pointer("/data/viewer").and_then(user_from_node));
    }
    if looks_like_uuid(value) {
        let by_id = api
            .graphql(json!({
                "query": "query($id: String!) { user(id: $id) { id url } }",
                "variables": { "id": value }
            }))
            .await?;
        return Ok(by_id.pointer("/data/user").and_then(user_from_node));
    }
    // Try email match first, then fall back to display-name lookup.
    let by_email = api
        .graphql(json!({
            "query": "query($email: String!) { users(filter: { email: { eq: $email } }, first: 1) { nodes { id url } } }",
            "variables": { "email": value }
        }))
        .await?;
    if let Some(user) = by_email
        .pointer("/data/users/nodes/0")
        .and_then(user_from_node)
    {
        return Ok(Some(user));
    }
    let Some(lookup) = api.lookup(LookupKind::User, value).await? else {
        return Ok(None);
    };
    let by_id = api
        .graphql(json!({
            "query": "query($id: String!) { user(id: $id) { id url } }",
            "variables": { "id": lookup.id }
        }))
        .await?;
    Ok(by_id.pointer("/data/user").and_then(user_from_node))
}

/// Required variant of [`resolve_user_opt`] — hard-fails with `NotFound` when
/// no user matches.
pub(super) async fn resolve_user_required(
    api: &LinearAgentApi,
    value: &str,
) -> Result<ResolvedUser, AdapterError> {
    resolve_user_opt(api, value).await?.ok_or_else(|| {
        SharedAdapterError::NotFound(format!("Linear user `{value}` not found")).into()
    })
}

/// Resolve assignee: `me` → viewer.id; UUID → passthrough; email or display
/// name → user lookup. Hard-fails on miss (used by issue update and explicit
/// create assignees).
pub(super) async fn resolve_assignee(
    api: &LinearAgentApi,
    value: Option<&str>,
) -> Result<Option<String>, AdapterError> {
    let Some(value) = value else { return Ok(None) };
    Ok(Some(resolve_user_required(api, value).await?.id))
}

/// Best-effort assignee default: resolve the calling moat user's email to a
/// Linear user. Returns `None` (and warns) when the caller has no email on
/// file or the email doesn't match a Linear member — never fails issue
/// creation.
pub(super) async fn default_assignee_from_caller(
    users: &dyn UserDisplay,
    api: &LinearAgentApi,
    user_id: &str,
) -> Option<String> {
    let Ok(uid) = user_id.parse::<uuid::Uuid>() else {
        return None;
    };
    let email = match users.email(&uid).await {
        Ok(Some(email)) => email,
        Ok(None) => {
            warn!(
                user_id,
                "assignee default: caller has no email; leaving issue unassigned"
            );
            return None;
        }
        Err(e) => {
            warn!(error = %e, user_id, "assignee default: failed to load caller; leaving issue unassigned");
            return None;
        }
    };
    match resolve_user_opt(api, &email).await {
        Ok(Some(u)) => Some(u.id),
        Ok(None) => {
            warn!(
                user_id,
                email,
                "assignee default: no Linear user matches caller email; leaving issue unassigned"
            );
            None
        }
        Err(e) => {
            warn!(error = %e, user_id, email, "assignee default: lookup failed; leaving issue unassigned");
            None
        }
    }
}

/// Resolve a parent value: UUID → as-is; identifier (SP-123) → fetch the
/// issue and return its UUID.
pub(super) async fn resolve_optional_issue(
    api: &LinearAgentApi,
    value: Option<&str>,
) -> Result<Option<String>, AdapterError> {
    let Some(value) = value else { return Ok(None) };
    if looks_like_uuid(value) {
        return Ok(Some(value.to_string()));
    }
    let issue = api.get_issue(value).await?;
    let id = issue.get("id").and_then(Value::as_str).ok_or_else(|| {
        SharedAdapterError::NotFound(format!("Linear issue `{value}` missing id"))
    })?;
    Ok(Some(id.to_string()))
}

/// Resolve a cycle value: UUID → as-is; "active" → the team's active
/// cycle; numeric string → cycle number on the team.
pub(super) async fn resolve_cycle_id(
    api: &LinearAgentApi,
    team_id: &str,
    value: Option<&str>,
) -> Result<Option<String>, AdapterError> {
    let Some(value) = value else { return Ok(None) };
    if looks_like_uuid(value) {
        return Ok(Some(value.to_string()));
    }
    if value == "active" {
        let active = api
            .graphql(json!({
                "query": "query($id: String!) { team(id: $id) { activeCycle { id } } }",
                "variables": { "id": team_id }
            }))
            .await?;
        return Ok(active
            .pointer("/data/team/activeCycle/id")
            .and_then(Value::as_str)
            .map(str::to_string));
    }
    let number: i64 = value.parse().map_err(|_| {
        SharedAdapterError::BadRequest(format!(
            "Invalid cycle value `{value}` (use a number or `active`)"
        ))
    })?;
    let by_number = api
        .graphql(json!({
            "query": "query($team: ID!, $number: Float!) { cycles(filter: { team: { id: { eq: $team } }, number: { eq: $number } }, first: 1) { nodes { id number } } }",
            "variables": { "team": team_id, "number": number }
        }))
        .await?;
    Ok(by_number
        .pointer("/data/cycles/nodes/0/id")
        .and_then(Value::as_str)
        .map(str::to_string))
}
