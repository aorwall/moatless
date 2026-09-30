use super::agent_api::{LinearAgentApi, LookupKind};
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_ports::identity::{ProviderIdentityLink, UserDisplay};
use serde_json::{Value, json};
use tracing::warn;

/// The `provider` a Linear user id is stored under in `user_provider_ids`.
pub(super) const LINEAR_PROVIDER: &str = moatless_types::adapter::AdapterKind::Linear.as_str();

/// Heuristic: a Linear UUID is 36 chars with dashes at positions 8/13/18/23.
/// Identifiers like `SP-123` start with letters and a dash. Anything that
/// parses as a UUID is treated as already-resolved.
pub(super) fn looks_like_uuid(s: &str) -> bool {
    uuid::Uuid::parse_str(s).is_ok()
}

/// Resolve a value that may be either a UUID (passthrough) or a name (lookup).
/// Returns `Ok(None)` for `None` input. `team_id` scopes the lookup — see
/// [`LinearAgentApi::lookup`] for what passing `None` costs.
pub(super) async fn resolve_optional(
    api: &LinearAgentApi,
    kind: LookupKind,
    value: Option<&str>,
    team_id: Option<&str>,
) -> Result<Option<String>, AdapterError> {
    let Some(value) = value else { return Ok(None) };
    if looks_like_uuid(value) {
        return Ok(Some(value.to_string()));
    }
    let lookup = api.lookup(kind, value, team_id).await?.ok_or_else(|| {
        SharedAdapterError::NotFound(format!("Linear {kind:?} `{value}` not found"))
    })?;
    Ok(Some(lookup.id))
}

/// Required variant of [`resolve_optional`].
pub(super) async fn resolve_required(
    api: &LinearAgentApi,
    kind: LookupKind,
    value: &str,
    team_id: Option<&str>,
) -> Result<String, AdapterError> {
    resolve_optional(api, kind, Some(value), team_id)
        .await?
        .ok_or_else(|| {
            SharedAdapterError::Internal("resolve_optional returned None unexpectedly".into())
                .into()
        })
}

/// The team a Linear issue belongs to, for scoping the state, project and cycle
/// names an update resolves against.
pub(super) async fn issue_team_id(
    api: &LinearAgentApi,
    id_or_key: &str,
) -> Result<String, AdapterError> {
    team_id_of(&api.get_issue(id_or_key).await?)
}

/// [`issue_team_id`] for a caller that already fetched the issue.
pub(super) fn team_id_of(issue: &Value) -> Result<String, AdapterError> {
    issue
        .pointer("/team/id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            SharedAdapterError::AdapterApi("Linear issue is missing team.id".into()).into()
        })
}

/// Resolve a team value (key like `SP`, or UUID). Lookup uses the team's `name`
/// field, so a team key like `SP` is checked via a key filter first.
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
        .lookup(LookupKind::Team, value, None)
        .await?
        .ok_or_else(|| SharedAdapterError::NotFound(format!("Linear team `{value}` not found")))?;
    Ok(lookup.id)
}

/// Return the default team id when the caller didn't specify one. If exactly
/// one team is accessible we use it; otherwise surface the list and require a
/// `--team` flag.
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

/// The Linear user id stored against a Moatless `login`, or `None` when there's
/// no `linear` mapping.
///
/// A store error is treated as a miss, not a failure — one branch of
/// [`resolve_user_opt`], and the branches after it can still match.
pub(super) async fn linear_id_for_login(
    identity: &dyn ProviderIdentityLink,
    login: &str,
) -> Option<String> {
    match identity.provider_id_for_login(LINEAR_PROVIDER, login).await {
        Ok(id) => id,
        Err(e) => {
            warn!(error = %e, login, "Linear id lookup for login failed; trying the remaining resolution steps");
            None
        }
    }
}

/// Resolve a user reference to a Linear user: `me` → viewer; UUID →
/// passthrough; Moatless login → its stored `linear` id; else email; else
/// display name.
///
/// The login step precedes email so a deliberately-set id outranks a
/// coincidental match; `Ok(None)` on no match lets callers decide whether a
/// miss is fatal or skippable.
pub(super) async fn resolve_user_opt(
    api: &LinearAgentApi,
    identity: &dyn ProviderIdentityLink,
    value: &str,
) -> Result<Option<ResolvedUser>, AdapterError> {
    if value == "me" {
        let viewer = api
            .graphql(json!({ "query": "query { viewer { id url } }" }))
            .await?;
        return Ok(viewer.pointer("/data/viewer").and_then(user_from_node));
    }
    if looks_like_uuid(value) {
        return user_by_id(api, value).await;
    }
    // A login is checked before the two Linear-side name matches: it is the
    // mapping someone set on purpose.
    if let Some(id) = linear_id_for_login(identity, value).await {
        if let Some(user) = user_by_id(api, &id).await? {
            return Ok(Some(user));
        }
        warn!(
            login = value,
            linear_id = id,
            "Linear id mapped to this login no longer names a Linear user; trying email and display name"
        );
    }
    user_by_email_or_name(api, value).await
}

/// Match `value` against Linear's own directory: email first, then display
/// name.
///
/// The half of [`resolve_user_opt`] that consults no Moatless mapping —
/// [`default_assignee_from_caller`] enters here directly, having already tried
/// the caller's login.
async fn user_by_email_or_name(
    api: &LinearAgentApi,
    value: &str,
) -> Result<Option<ResolvedUser>, AdapterError> {
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
    let Some(lookup) = api.lookup(LookupKind::User, value, None).await? else {
        return Ok(None);
    };
    user_by_id(api, &lookup.id).await
}

/// Fetch one Linear user by its UUID. `Ok(None)` when Linear knows no such id.
async fn user_by_id(api: &LinearAgentApi, id: &str) -> Result<Option<ResolvedUser>, AdapterError> {
    let by_id = api
        .graphql(json!({
            "query": "query($id: String!) { user(id: $id) { id url } }",
            "variables": { "id": id }
        }))
        .await?;
    Ok(by_id.pointer("/data/user").and_then(user_from_node))
}

/// Required variant of [`resolve_user_opt`] — hard-fails with `NotFound` when
/// no user matches.
pub(super) async fn resolve_user_required(
    api: &LinearAgentApi,
    identity: &dyn ProviderIdentityLink,
    value: &str,
) -> Result<ResolvedUser, AdapterError> {
    resolve_user_opt(api, identity, value)
        .await?
        .ok_or_else(|| {
            SharedAdapterError::NotFound(format!("Linear user `{value}` not found")).into()
        })
}

/// Resolve an assignee through [`resolve_user_opt`], hard-failing on a miss.
/// Used by issue update and by an explicit create assignee.
pub(super) async fn resolve_assignee(
    api: &LinearAgentApi,
    identity: &dyn ProviderIdentityLink,
    value: Option<&str>,
) -> Result<Option<String>, AdapterError> {
    let Some(value) = value else { return Ok(None) };
    Ok(Some(resolve_user_required(api, identity, value).await?.id))
}

/// Resolve a delegate the same way as [`resolve_assignee`], with `"null"` and
/// `""` clearing the field. Follows the `parent` convention in
/// `issue_commands`, because undelegating is how a lane releases a claim.
pub(super) async fn resolve_delegate(
    api: &LinearAgentApi,
    identity: &dyn ProviderIdentityLink,
    value: Option<&str>,
) -> Result<Option<serde_json::Value>, AdapterError> {
    match value {
        None => Ok(None),
        Some("null") | Some("") => Ok(Some(serde_json::Value::Null)),
        Some(v) => Ok(Some(serde_json::Value::String(
            resolve_user_required(api, identity, v).await?.id,
        ))),
    }
}

/// Best-effort assignee default: the caller's Linear user, from their stored
/// `linear` provider id, falling back to matching their moat email against
/// Linear's members.
///
/// The provider id is the reliable half; an email only matches when both
/// directories agree. Returns `None` (and warns) when neither resolves — never
/// fails issue creation.
pub(super) async fn default_assignee_from_caller(
    users: &dyn UserDisplay,
    identity: &dyn ProviderIdentityLink,
    api: &LinearAgentApi,
    user_id: &str,
    login: &str,
) -> Option<String> {
    if let Some(id) = linear_id_for_login(identity, login).await {
        match user_by_id(api, &id).await {
            Ok(Some(u)) => return Some(u.id),
            Ok(None) => warn!(
                login,
                linear_id = id,
                "assignee default: the login's Linear id names no Linear user; falling back to email"
            ),
            Err(e) => {
                warn!(error = %e, login, linear_id = id, "assignee default: Linear id lookup failed; falling back to email");
            }
        }
    }
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
    match user_by_email_or_name(api, &email).await {
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

/// The caller's Moatless display name, for `issueCreate`'s `createAsUser`.
/// Falls back to `login` on every miss, so it never fails issue creation.
pub(super) async fn creator_name(users: &dyn UserDisplay, user_id: &str, login: &str) -> String {
    let Ok(uid) = user_id.parse::<uuid::Uuid>() else {
        return login.to_string();
    };
    match users.name(&uid).await {
        Ok(Some(name)) => name,
        Ok(None) => login.to_string(),
        Err(e) => {
            warn!(error = %e, user_id, "creator name: failed to load caller; using the login");
            login.to_string()
        }
    }
}

/// Resolve a parent value: UUID → as-is; identifier (SP-123) → fetch the issue
/// and return its UUID.
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

/// Resolve a cycle value: UUID → as-is; `"active"` → the team's active cycle;
/// numeric string → cycle number on the team.
///
/// Only a name reads `team_id`, so a caller fetching the team lazily may pass
/// `None` for a UUID; `None` for a name is a caller bug and fails here.
pub(super) async fn resolve_cycle_id(
    api: &LinearAgentApi,
    team_id: Option<&str>,
    value: Option<&str>,
) -> Result<Option<String>, AdapterError> {
    let Some(value) = value else { return Ok(None) };
    if looks_like_uuid(value) {
        return Ok(Some(value.to_string()));
    }
    let team_id = team_id.ok_or_else(|| {
        SharedAdapterError::Internal(format!("Linear cycle `{value}` needs a team to resolve in"))
    })?;
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use moatless_adapter_model::test_support::ProviderStub;
    use moatless_ports::identity::{
        AppToken, MockAppCredentials, MockProviderIdentityLink, MockUserDisplay,
    };
    use moatless_types::store::StoreError;
    use serde_json::json;

    use super::super::agent_api::LinearAgentApi;
    use super::{
        LINEAR_PROVIDER, creator_name, default_assignee_from_caller, linear_id_for_login,
        resolve_user_opt,
    };

    /// The Linear id stored against the login, and the answer the precedence
    /// tests expect back.
    const MAPPED_ID: &str = "11111111-2222-3333-4444-555555555555";
    /// The Linear id the directory returns for an email or display-name match.
    /// A different user, so which branch ran is readable off the result.
    const DIRECTORY_ID: &str = "99999999-8888-7777-6666-555555555555";

    /// A client whose whole API host is `base`, with a token that never needs
    /// refreshing so nothing but the query under test reaches the stub.
    fn api_at(base: &str) -> LinearAgentApi {
        let mut tokens = MockAppCredentials::new();
        tokens.expect_app_token().returning(|_| {
            Ok(Some(AppToken {
                access_token: "lin_test".into(),
                refresh_token: None,
                token_type: None,
                expires_at: Some(chrono::Utc::now() + chrono::Duration::days(1)),
                metadata: None,
            }))
        });
        LinearAgentApi::with_base_url(
            crate::agent_api::http_client(moatless_core::config::HttpConfig::default().linear),
            Arc::new(tokens),
            "test-org".to_string(),
            None,
            None,
            base,
        )
    }

    /// A Linear that answers every user query at once: `user(id:)` with
    /// [`MAPPED_ID`], `users(filter: { email: })` with [`DIRECTORY_ID`].
    ///
    /// Both branches can succeed, so the id that comes back names the branch
    /// that ran.
    async fn stub_answering_every_user_query() -> ProviderStub {
        ProviderStub::start(|_path| {
            json!({
                "data": {
                    "user": { "id": MAPPED_ID, "url": "https://linear.app/u/mapped" },
                    "users": { "nodes": [{ "id": DIRECTORY_ID, "url": "https://linear.app/u/directory" }] }
                }
            })
        })
        .await
    }

    /// The branch order this change exists for: a stored id beats an email or
    /// display-name match, and costs one call rather than running both.
    #[tokio::test]
    async fn a_stored_id_outranks_a_directory_match() {
        let stub = stub_answering_every_user_query().await;
        let api = api_at(stub.base_url());
        let mut identity = MockProviderIdentityLink::new();
        identity
            .expect_provider_id_for_login()
            .returning(|_, _| Ok(Some(MAPPED_ID.into())));

        let user = resolve_user_opt(&api, &identity, "aorwall")
            .await
            .expect("stub answers the query")
            .expect("the mapped id resolves");

        assert_eq!(user.id, MAPPED_ID);
        assert_eq!(stub.graphql_operations(), vec!["user".to_string()]);
    }

    /// An unmapped login still resolves the way it did before this change.
    #[tokio::test]
    async fn an_unmapped_login_falls_through_to_the_directory() {
        let stub = stub_answering_every_user_query().await;
        let api = api_at(stub.base_url());
        let mut identity = MockProviderIdentityLink::new();
        identity
            .expect_provider_id_for_login()
            .returning(|_, _| Ok(None));

        let user = resolve_user_opt(&api, &identity, "someone")
            .await
            .expect("stub answers the query")
            .expect("the email match resolves");

        assert_eq!(user.id, DIRECTORY_ID);
        assert_eq!(stub.graphql_operations(), vec!["users".to_string()]);
    }

    /// The assignee default prefers the caller's stored id, and never loads
    /// their email when it has one.
    #[tokio::test]
    async fn the_assignee_default_prefers_the_stored_id() {
        let stub = stub_answering_every_user_query().await;
        let api = api_at(stub.base_url());
        let mut identity = MockProviderIdentityLink::new();
        identity
            .expect_provider_id_for_login()
            .returning(|_, _| Ok(Some(MAPPED_ID.into())));
        let mut users = MockUserDisplay::new();
        users.expect_email().never();

        let assignee = default_assignee_from_caller(
            &users,
            &identity,
            &api,
            "00000000-0000-4000-8000-000000000001",
            "aorwall",
        )
        .await;

        assert_eq!(assignee.as_deref(), Some(MAPPED_ID));
        assert_eq!(stub.graphql_operations(), vec!["user".to_string()]);
    }

    /// With no stored id the default falls back to the caller's email, and the
    /// fallback does not re-enter the login branch it already tried.
    #[tokio::test]
    async fn the_assignee_default_falls_back_to_the_caller_email() {
        let stub = stub_answering_every_user_query().await;
        let api = api_at(stub.base_url());
        let mut identity = MockProviderIdentityLink::new();
        identity
            .expect_provider_id_for_login()
            .times(1)
            .returning(|_, _| Ok(None));
        let mut users = MockUserDisplay::new();
        users
            .expect_email()
            .returning(|_| Ok(Some("albert@example.com".into())));

        let assignee = default_assignee_from_caller(
            &users,
            &identity,
            &api,
            "00000000-0000-4000-8000-000000000001",
            "aorwall",
        )
        .await;

        assert_eq!(assignee.as_deref(), Some(DIRECTORY_ID));
        assert_eq!(stub.graphql_operations(), vec!["users".to_string()]);
    }

    #[tokio::test]
    async fn login_resolves_to_its_stored_linear_id() {
        let mut identity = MockProviderIdentityLink::new();
        identity
            .expect_provider_id_for_login()
            .withf(|provider, login| provider == LINEAR_PROVIDER && login == "aorwall")
            .returning(|_, _| Ok(Some("11111111-2222-3333-4444-555555555555".into())));

        assert_eq!(
            linear_id_for_login(&identity, "aorwall").await.as_deref(),
            Some("11111111-2222-3333-4444-555555555555")
        );
    }

    #[tokio::test]
    async fn unmapped_login_is_a_miss() {
        let mut identity = MockProviderIdentityLink::new();
        identity
            .expect_provider_id_for_login()
            .returning(|_, _| Ok(None));

        assert_eq!(linear_id_for_login(&identity, "someone").await, None);
    }

    /// A store failure must fall through to the email and display-name steps
    /// rather than fail the request: the caller may still be resolvable.
    #[tokio::test]
    async fn store_error_is_a_miss() {
        let mut identity = MockProviderIdentityLink::new();
        identity
            .expect_provider_id_for_login()
            .returning(|_, _| Err(StoreError::new("database is down")));

        assert_eq!(linear_id_for_login(&identity, "aorwall").await, None);
    }

    const CALLER_ID: &str = "00000000-0000-4000-8000-000000000001";

    #[tokio::test]
    async fn the_creator_is_the_callers_display_name() {
        let mut users = MockUserDisplay::new();
        users
            .expect_name()
            .withf(|id| id.to_string() == CALLER_ID)
            .returning(|_| Ok(Some("Albert Örwall".into())));

        assert_eq!(
            creator_name(&users, CALLER_ID, "aorwall").await,
            "Albert Örwall"
        );
    }

    /// The creator name must never fail issue creation: every miss is the login.
    #[tokio::test]
    async fn the_creator_falls_back_to_the_login() {
        let mut unknown = MockUserDisplay::new();
        unknown.expect_name().returning(|_| Ok(None));
        let mut failing = MockUserDisplay::new();
        failing
            .expect_name()
            .returning(|_| Err(StoreError::new("database is down")));
        let mut unread = MockUserDisplay::new();
        unread.expect_name().never();

        assert_eq!(
            creator_name(&unknown, CALLER_ID, "aorwall").await,
            "aorwall"
        );
        assert_eq!(
            creator_name(&failing, CALLER_ID, "aorwall").await,
            "aorwall"
        );
        assert_eq!(
            creator_name(&unread, "not-a-uuid", "aorwall").await,
            "aorwall"
        );
    }
}
