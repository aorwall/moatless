//! Task credential resolution for sandbox runtime consumers.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::Arc;

use tracing::warn;

use crate::auth::github_app_service::GitHubAppService;
use crate::auth::provider_token_dao::ProviderTokenRepository;
use crate::auth::provider_tokens::ProviderTokenAuthRepository;
use crate::auth::token_refresh::ensure_fresh_token;
use crate::sandbox::models::ServerConfig;
use crate::secret::error::SecretError;
use crate::secret::models::{ResolvedSecret, SecretScopeType, is_reserved_key};
use crate::secret::service::SecretService;
use crate::task::dao::TaskRepository;

pub const GITHUB_TOKEN: &str = "GITHUB_TOKEN";
pub const GITLAB_TOKEN: &str = "GITLAB_TOKEN";
pub const GITNESS_TOKEN: &str = "GITNESS_TOKEN";
pub const CODEX_AUTH_JSON: &str = "CODEX_AUTH_JSON";
pub const OPENAI_API_KEY: &str = "OPENAI_API_KEY";
pub const ANTHROPIC_API_KEY: &str = "ANTHROPIC_API_KEY";
pub const CLAUDE_CODE_OAUTH_TOKEN: &str = "CLAUDE_CODE_OAUTH_TOKEN";

/// Plaintext server-targeted secrets for one provision request.
///
/// This type is internal-only and redacts values in Debug output.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ServerSecretBundle {
    by_server: BTreeMap<String, BTreeMap<String, String>>,
}

impl ServerSecretBundle {
    pub fn is_empty(&self) -> bool {
        self.by_server.is_empty()
    }

    pub fn get(&self, server_name: &str) -> Option<&BTreeMap<String, String>> {
        self.by_server.get(server_name)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &BTreeMap<String, String>)> {
        self.by_server.iter()
    }

    pub fn secret_count(&self) -> usize {
        self.by_server.values().map(BTreeMap::len).sum()
    }
}

impl fmt::Debug for ServerSecretBundle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let counts: BTreeMap<&str, usize> = self
            .by_server
            .iter()
            .map(|(server, secrets)| (server.as_str(), secrets.len()))
            .collect();
        f.debug_struct("ServerSecretBundle")
            .field("server_secret_counts", &counts)
            .finish()
    }
}

pub struct ServerSecretInputs<'a> {
    pub task_id: &'a str,
    pub user_id: &'a str,
    pub team_id: Option<&'a str>,
    pub secrets: Option<&'a Arc<SecretService>>,
    pub servers: &'a [ServerConfig],
}

pub struct TaskCredentialInputs<'a> {
    pub task_id: &'a str,
    pub user_id: &'a str,
    pub secrets: Option<&'a Arc<SecretService>>,
    pub provider_tokens: Option<&'a Arc<dyn ProviderTokenRepository>>,
    pub tasks: Option<&'a Arc<dyn TaskRepository>>,
    pub github_app_service: Option<&'a Arc<GitHubAppService>>,
    pub provider_token_auth: Option<&'a Arc<dyn ProviderTokenAuthRepository>>,
}

pub async fn resolve_server_secrets(
    input: ServerSecretInputs<'_>,
) -> Result<ServerSecretBundle, SecretError> {
    let Some(secrets) = input.secrets else {
        return Ok(ServerSecretBundle::default());
    };
    if input.servers.is_empty() {
        return Ok(ServerSecretBundle::default());
    }

    let mut targets = vec!["all".to_string(), "all_servers".to_string()];
    targets.extend(
        input
            .servers
            .iter()
            .map(|server| format!("server:{}", server.name)),
    );

    let mut candidates = Vec::new();
    candidates.extend(
        resolve_server_secret_scope(
            secrets,
            input.user_id,
            SecretScopeType::Global,
            None,
            &targets,
            ScopeRank::Global,
        )
        .await?,
    );
    if let Some(team_id) = input.team_id {
        candidates.extend(
            resolve_server_secret_scope(
                secrets,
                input.user_id,
                SecretScopeType::Team,
                Some(team_id),
                &targets,
                ScopeRank::Team,
            )
            .await?,
        );
    }
    candidates.extend(
        resolve_server_secret_scope(
            secrets,
            input.user_id,
            SecretScopeType::User,
            Some(input.user_id),
            &targets,
            ScopeRank::User,
        )
        .await?,
    );

    let bundle = build_server_secret_bundle(input.servers, candidates);
    if !bundle.is_empty() {
        tracing::info!(
            task_id = input.task_id,
            server_count = bundle.by_server.len(),
            secret_count = bundle.secret_count(),
            "Resolved server-targeted secrets"
        );
    }
    Ok(bundle)
}

pub async fn resolve_task_credentials(input: TaskCredentialInputs<'_>) -> HashMap<String, String> {
    let team_id = resolve_task_team_id(input.tasks, input.task_id).await;
    let mut credentials = resolve_scoped_secrets(
        input.secrets,
        input.user_id,
        team_id.as_deref(),
        input.task_id,
    )
    .await;

    fill_provider_tokens(
        &mut credentials,
        input.provider_tokens,
        team_id.as_deref(),
        input.user_id,
        input.github_app_service,
        input.provider_token_auth,
    )
    .await;

    credentials
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ScopeRank {
    Global,
    Team,
    User,
}

struct ScopedServerSecret {
    secret: ResolvedSecret,
    scope_rank: ScopeRank,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum TargetSpecificity {
    Broad,
    Server,
}

struct SelectedServerSecret {
    value: String,
    specificity: TargetSpecificity,
    scope_rank: ScopeRank,
}

async fn resolve_server_secret_scope(
    secrets: &Arc<SecretService>,
    actor_id: &str,
    scope_type: SecretScopeType,
    scope_id: Option<&str>,
    targets: &[String],
    scope_rank: ScopeRank,
) -> Result<Vec<ScopedServerSecret>, SecretError> {
    let resolved = secrets
        .resolve_plaintext_scope(actor_id, scope_type, scope_id, targets)
        .await?;
    Ok(resolved
        .into_iter()
        .map(|secret| ScopedServerSecret { secret, scope_rank })
        .collect())
}

fn build_server_secret_bundle(
    servers: &[ServerConfig],
    candidates: Vec<ScopedServerSecret>,
) -> ServerSecretBundle {
    let server_names = servers
        .iter()
        .map(|server| server.name.as_str())
        .collect::<Vec<_>>();
    let mut selected: BTreeMap<String, BTreeMap<String, SelectedServerSecret>> = BTreeMap::new();

    for candidate in candidates {
        if is_reserved_key(&candidate.secret.key) {
            warn!(
                target = %candidate.secret.target,
                key = %candidate.secret.key,
                "Skipping reserved server-targeted secret key"
            );
            continue;
        }

        match server_secret_target(candidate.secret.target.as_str(), &server_names) {
            ServerSecretTarget::AllServers => {
                for server_name in &server_names {
                    select_server_secret(
                        &mut selected,
                        server_name,
                        &candidate.secret.key,
                        &candidate.secret.value,
                        TargetSpecificity::Broad,
                        candidate.scope_rank,
                    );
                }
            }
            ServerSecretTarget::OneServer(server_name) => {
                select_server_secret(
                    &mut selected,
                    server_name,
                    &candidate.secret.key,
                    &candidate.secret.value,
                    TargetSpecificity::Server,
                    candidate.scope_rank,
                );
            }
            ServerSecretTarget::NotServer => {}
        }
    }

    let by_server = selected
        .into_iter()
        .filter_map(|(server, secrets)| {
            let values = secrets
                .into_iter()
                .map(|(key, selected)| (key, selected.value))
                .collect::<BTreeMap<_, _>>();
            (!values.is_empty()).then_some((server, values))
        })
        .collect();

    ServerSecretBundle { by_server }
}

enum ServerSecretTarget<'a> {
    AllServers,
    OneServer(&'a str),
    NotServer,
}

fn server_secret_target<'a>(target: &'a str, server_names: &[&'a str]) -> ServerSecretTarget<'a> {
    match target {
        "all" | "all_servers" => ServerSecretTarget::AllServers,
        _ => {
            let Some(server_name) = target.strip_prefix("server:") else {
                return ServerSecretTarget::NotServer;
            };
            if server_names.contains(&server_name) {
                ServerSecretTarget::OneServer(server_name)
            } else {
                ServerSecretTarget::NotServer
            }
        }
    }
}

fn select_server_secret(
    selected: &mut BTreeMap<String, BTreeMap<String, SelectedServerSecret>>,
    server_name: &str,
    key: &str,
    value: &str,
    specificity: TargetSpecificity,
    scope_rank: ScopeRank,
) {
    let server = selected.entry(server_name.to_string()).or_default();
    match server.get(key) {
        Some(existing)
            if existing.specificity > specificity
                || (existing.specificity == specificity && existing.scope_rank <= scope_rank) =>
        {
            return;
        }
        _ => {}
    }

    server.insert(
        key.to_string(),
        SelectedServerSecret {
            value: value.to_string(),
            specificity,
            scope_rank,
        },
    );
}

async fn resolve_task_team_id(
    tasks: Option<&Arc<dyn TaskRepository>>,
    task_id: &str,
) -> Option<String> {
    let tasks = tasks?;
    match tasks.find_by_id_internal(task_id).await {
        Ok(Some(task)) => task.team_id,
        Ok(None) => {
            warn!(task_id, "resolve_task_team_id: task not found");
            None
        }
        Err(e) => {
            warn!(task_id, error = %e, "resolve_task_team_id: DB error");
            None
        }
    }
}

async fn resolve_scoped_secrets(
    secrets: Option<&Arc<SecretService>>,
    user_id: &str,
    team_id: Option<&str>,
    task_id: &str,
) -> HashMap<String, String> {
    let Some(secrets) = secrets else {
        return HashMap::new();
    };

    let targets = vec!["agent".to_string(), "all".to_string()];
    match secrets
        .resolve_accessible_plaintext(user_id, team_id, &targets)
        .await
    {
        Ok(credentials) => credentials,
        Err(e) => {
            warn!(task_id, error = %e, "Failed to resolve scoped secrets");
            HashMap::new()
        }
    }
}

async fn fill_provider_tokens(
    credentials: &mut HashMap<String, String>,
    provider_tokens: Option<&Arc<dyn ProviderTokenRepository>>,
    team_id: Option<&str>,
    user_id: &str,
    github_app_service: Option<&Arc<GitHubAppService>>,
    provider_token_auth: Option<&Arc<dyn ProviderTokenAuthRepository>>,
) {
    let Some(provider_tokens) = provider_tokens else {
        return;
    };

    if let Some(team_id) = team_id {
        fill_team_provider_tokens(credentials, provider_tokens, team_id).await;
    }
    fill_user_provider_tokens(
        credentials,
        provider_tokens,
        user_id,
        github_app_service,
        provider_token_auth,
    )
    .await;
}

async fn fill_team_provider_tokens(
    credentials: &mut HashMap<String, String>,
    provider_tokens: &Arc<dyn ProviderTokenRepository>,
    team_id: &str,
) {
    for (provider, cred_key) in [
        ("github", GITHUB_TOKEN),
        ("gitlab", GITLAB_TOKEN),
        ("gitness", GITNESS_TOKEN),
        ("codex", CODEX_AUTH_JSON),
        ("openai", OPENAI_API_KEY),
    ] {
        if credentials.contains_key(cred_key) {
            continue;
        }
        match provider_tokens.find_team_token(team_id, provider).await {
            Ok(Some(row)) => {
                credentials.insert(cred_key.to_string(), row.access_token);
            }
            Ok(None) => {}
            Err(e) => {
                warn!(error = %e, team_id, provider, "Failed to query team provider_token");
            }
        }
    }

    if credentials.contains_key(CLAUDE_CODE_OAUTH_TOKEN)
        || credentials.contains_key(ANTHROPIC_API_KEY)
    {
        return;
    }
    match provider_tokens.find_team_token(team_id, "claude").await {
        Ok(Some(row)) => {
            credentials.insert(CLAUDE_CODE_OAUTH_TOKEN.to_string(), row.access_token);
        }
        Ok(None) => {}
        Err(e) => {
            warn!(error = %e, team_id, "Failed to query team claude provider_token");
        }
    }
}

async fn fill_user_provider_tokens(
    credentials: &mut HashMap<String, String>,
    provider_tokens: &Arc<dyn ProviderTokenRepository>,
    user_id: &str,
    github_app_service: Option<&Arc<GitHubAppService>>,
    provider_token_auth: Option<&Arc<dyn ProviderTokenAuthRepository>>,
) {
    if !credentials.contains_key(GITHUB_TOKEN) {
        let resolved = if let (Some(github_app), Some(token_auth)) =
            (github_app_service, provider_token_auth)
        {
            match ensure_fresh_token(
                provider_tokens.as_ref(),
                token_auth.as_ref(),
                github_app,
                user_id,
            )
            .await
            {
                Ok(token) => Some(token),
                Err(e) => {
                    warn!(
                        error = %e,
                        user_id,
                        "GitHub App token refresh failed, falling back to raw token"
                    );
                    provider_tokens
                        .find_provider_token(user_id, "github")
                        .await
                        .ok()
                        .flatten()
                        .map(|row| row.access_token)
                }
            }
        } else {
            provider_tokens
                .find_provider_token(user_id, "github")
                .await
                .ok()
                .flatten()
                .map(|row| row.access_token)
        };

        if let Some(token) = resolved {
            credentials.insert(GITHUB_TOKEN.to_string(), token);
        }
    }

    for (provider, cred_key) in [
        ("gitlab", GITLAB_TOKEN),
        ("gitness", GITNESS_TOKEN),
        ("codex", CODEX_AUTH_JSON),
        ("openai", OPENAI_API_KEY),
    ] {
        if credentials.contains_key(cred_key) {
            continue;
        }
        match provider_tokens.find_provider_token(user_id, provider).await {
            Ok(Some(row)) => {
                credentials.insert(cred_key.to_string(), row.access_token);
            }
            Ok(None) => {}
            Err(e) => {
                warn!(error = %e, user_id, provider, "Failed to query provider_token");
            }
        }
    }

    if credentials.contains_key(CLAUDE_CODE_OAUTH_TOKEN)
        || credentials.contains_key(ANTHROPIC_API_KEY)
    {
        return;
    }
    match provider_tokens.find_provider_token(user_id, "claude").await {
        Ok(Some(row)) => {
            credentials.insert(CLAUDE_CODE_OAUTH_TOKEN.to_string(), row.access_token);
        }
        Ok(None) => {}
        Err(e) => {
            warn!(error = %e, user_id, "Failed to query claude provider_token");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(name: &str) -> ServerConfig {
        ServerConfig {
            name: name.to_string(),
            ..Default::default()
        }
    }

    fn scoped_secret(
        target: &str,
        key: &str,
        value: &str,
        scope_rank: ScopeRank,
    ) -> ScopedServerSecret {
        ScopedServerSecret {
            secret: ResolvedSecret {
                key: key.to_string(),
                value: value.to_string(),
                target: target.to_string(),
                provider: None,
            },
            scope_rank,
        }
    }

    #[test]
    fn all_servers_target_applies_to_every_server() {
        let bundle = build_server_secret_bundle(
            &[server("web"), server("api")],
            vec![scoped_secret(
                "all_servers",
                "STRIPE_SECRET_KEY",
                "secret",
                ScopeRank::Global,
            )],
        );

        assert_eq!(
            bundle.get("web").unwrap().get("STRIPE_SECRET_KEY"),
            Some(&"secret".to_string())
        );
        assert_eq!(
            bundle.get("api").unwrap().get("STRIPE_SECRET_KEY"),
            Some(&"secret".to_string())
        );
    }

    #[test]
    fn server_specific_target_overrides_broad_target() {
        let bundle = build_server_secret_bundle(
            &[server("web"), server("api")],
            vec![
                scoped_secret(
                    "all_servers",
                    "STRIPE_SECRET_KEY",
                    "global",
                    ScopeRank::Global,
                ),
                scoped_secret(
                    "server:web",
                    "STRIPE_SECRET_KEY",
                    "team-web",
                    ScopeRank::Team,
                ),
            ],
        );

        assert_eq!(
            bundle.get("web").unwrap().get("STRIPE_SECRET_KEY"),
            Some(&"team-web".to_string())
        );
        assert_eq!(
            bundle.get("api").unwrap().get("STRIPE_SECRET_KEY"),
            Some(&"global".to_string())
        );
    }

    #[test]
    fn higher_precedence_scope_wins_for_same_target_specificity() {
        let bundle = build_server_secret_bundle(
            &[server("web")],
            vec![
                scoped_secret(
                    "server:web",
                    "STRIPE_SECRET_KEY",
                    "global-web",
                    ScopeRank::Global,
                ),
                scoped_secret(
                    "server:web",
                    "STRIPE_SECRET_KEY",
                    "team-web",
                    ScopeRank::Team,
                ),
            ],
        );

        assert_eq!(
            bundle.get("web").unwrap().get("STRIPE_SECRET_KEY"),
            Some(&"global-web".to_string())
        );
    }

    #[test]
    fn reserved_keys_are_not_delivered_to_servers() {
        let bundle = build_server_secret_bundle(
            &[server("web")],
            vec![scoped_secret(
                "server:web",
                "USER_AUTH_TOKEN",
                "secret",
                ScopeRank::Global,
            )],
        );

        assert!(bundle.is_empty());
    }

    #[test]
    fn server_secret_bundle_debug_redacts_values() {
        let bundle = build_server_secret_bundle(
            &[server("web")],
            vec![scoped_secret(
                "server:web",
                "STRIPE_SECRET_KEY",
                "super-secret",
                ScopeRank::Global,
            )],
        );

        let debug = format!("{bundle:?}");
        assert!(debug.contains("server_secret_counts"));
        assert!(!debug.contains("super-secret"));
    }
}
