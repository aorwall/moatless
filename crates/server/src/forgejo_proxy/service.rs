//! Which Forgejo request a sandbox may send, and with whose token.

use std::sync::Arc;

use moatless_core::proxy_rules::NormalPath;
use moatless_ports::identity::{GitHostTokenError, GitHostTokenIssuer};
use moatless_sandbox_control::secrets::resolve_task_actor;
use moatless_types::git_host::GitHostAccount;
use moatless_types::http::HttpStreamTimeouts;
use moatless_workspace_ports::WorkspaceResolver;
use tracing::warn;

use crate::proxy::caller::{CallerError, SandboxCallers, SandboxScope};

/// The only API this proxy forwards to; anything else on the host is refused.
const API_PREFIX: &str = "/api/v1/";

#[derive(Debug, thiserror::Error)]
pub enum ForgejoProxyError {
    /// Every rejection of the caller's own task token.
    #[error("{0}")]
    Unauthorized(String),
    /// A host, path or parameter this proxy does not forward.
    #[error("{0}")]
    Refused(String),
    /// The run-as identity holds no usable credential for the host. Shown to
    /// the caller, so it names what to connect.
    #[error("{0}")]
    NoCredential(String),
    /// A store or the upstream failed; the caller's request may succeed later.
    #[error("{0}")]
    Upstream(String),
}

/// Resolves a proxied Forgejo API request to an upstream URL and a token.
pub struct ForgejoProxy {
    callers: Arc<SandboxCallers>,
    workspaces: Arc<dyn WorkspaceResolver>,
    git_host_tokens: Arc<dyn GitHostTokenIssuer>,
    /// Replaces `https://{host}` in tests, which serve plain HTTP on loopback.
    base_override: Option<String>,
    pub(crate) http: reqwest::Client,
}

/// Idle, not total: an archive download streams for as long as it is big.
///
/// Redirects are returned rather than followed, so the token only ever goes to
/// the host the request names.
#[expect(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "sets the timeouts `Config::http` holds, and fails as `Client::new` would"
)]
fn http_client(timeouts: HttpStreamTimeouts) -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(timeouts.connect)
        .read_timeout(timeouts.idle)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(moatless_core::USER_AGENT)
        .build()
        .expect("the TLS backend initialises")
}

impl ForgejoProxy {
    pub(crate) fn new(
        callers: Arc<SandboxCallers>,
        workspaces: Arc<dyn WorkspaceResolver>,
        git_host_tokens: Arc<dyn GitHostTokenIssuer>,
        timeouts: HttpStreamTimeouts,
    ) -> Self {
        Self {
            callers,
            workspaces,
            git_host_tokens,
            base_override: None,
            http: http_client(timeouts),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_base(mut self, base: &str) -> Self {
        self.base_override = Some(base.trim_end_matches('/').to_string());
        self
    }

    /// The upstream URL for `path_and_query` on `host`, which [`checked`] has
    /// already passed.
    pub fn upstream_url(&self, host: &str, path_and_query: &str) -> String {
        match &self.base_override {
            Some(base) => format!("{base}{path_and_query}"),
            None => format!(
                "{}{path_and_query}",
                moatless_identity::forgejo::instances::base_url(host)
            ),
        }
    }

    /// Verify `task_token` and read the sandbox it was minted for.
    pub async fn authenticate(
        &self,
        task_token: Option<&str>,
    ) -> Result<SandboxScope, ForgejoProxyError> {
        self.callers
            .authenticate(task_token)
            .await
            .map_err(|e| match e {
                CallerError::Unauthorized(_) | CallerError::SandboxGone => {
                    ForgejoProxyError::Unauthorized(e.to_string())
                }
                CallerError::Upstream(message) => ForgejoProxyError::Upstream(message),
            })
    }

    /// The run-as identity's credential for `host`, resolved per request.
    ///
    /// The credential is keyed by `host`, so a pod naming another host gets
    /// that host's credential or none: no token is sent anywhere it was not
    /// issued for.
    pub async fn token(
        &self,
        scope: &SandboxScope,
        host: &str,
    ) -> Result<String, ForgejoProxyError> {
        let actor = resolve_task_actor(
            Some(&self.workspaces),
            Some(&scope.workspace),
            scope.run_as.as_deref(),
            &scope.user,
        )
        .await;
        let account = GitHostAccount::Forgejo {
            host: host.to_string(),
        };
        self.git_host_tokens
            .token(&actor, &account)
            .await
            .map_err(|error| credential_error(&error, host))
    }
}

fn credential_error(error: &GitHostTokenError, host: &str) -> ForgejoProxyError {
    match error {
        GitHostTokenError::NoCredential { .. } => ForgejoProxyError::NoCredential(format!(
            "{error}. Connect Forgejo at {host} under Settings → Version control; \
             for a bot user an admin runs `moat users set-token <bot> --provider forgejo --host {host}`"
        )),
        GitHostTokenError::Forbidden { .. } | GitHostTokenError::Broken { .. } => {
            ForgejoProxyError::NoCredential(error.to_string())
        }
        GitHostTokenError::Store(_) => {
            warn!(%error, host, "Failed to resolve a Forgejo credential for a proxied request");
            ForgejoProxyError::Upstream(format!(
                "Failed to resolve the Forgejo credential for {host}"
            ))
        }
    }
}

/// `host` lowercased, if it is a bare DNS name.
///
/// Deliberately no port, userinfo or path: the credential lookup and the URL
/// are both built from it, and must name the same host.
pub fn host(raw: &str) -> Result<String, ForgejoProxyError> {
    let host = raw.to_ascii_lowercase();
    let label_ok = |label: &str| {
        !label.is_empty()
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    };
    if host.len() <= 253 && host.contains('.') && host.split('.').all(label_ok) {
        Ok(host)
    } else {
        Err(ForgejoProxyError::Refused(format!(
            "`{raw}` is not a Forgejo host name"
        )))
    }
}

/// `path_and_query` if this proxy forwards it.
///
/// Refuses the site-administration API and every [`WITHHELD_PARAMETERS`] key:
/// a sandbox acts as its run-as identity and no one else. The `Sudo` header
/// is dropped in [`super::handlers`] for the same reason.
pub fn checked(path_and_query: &str) -> Result<(), ForgejoProxyError> {
    let path = NormalPath::parse(path_and_query)
        .map_err(|e| ForgejoProxyError::Refused(format!("Refused the path: {e}")))?;
    let path = path.as_str().to_ascii_lowercase();
    if !path.starts_with(API_PREFIX) {
        return Err(ForgejoProxyError::Refused(
            "Only the Forgejo API under /api/v1/ is proxied".to_string(),
        ));
    }
    let rest = &path[API_PREFIX.len()..];
    if rest == "admin" || rest.starts_with("admin/") {
        return Err(ForgejoProxyError::Refused(
            "The Forgejo site-administration API is not proxied".to_string(),
        ));
    }
    if let Some(key) = withheld_parameter(path_and_query) {
        return Err(ForgejoProxyError::Refused(format!(
            "Forgejo's `{key}` query parameter is not proxied"
        )));
    }
    Ok(())
}

/// Query keys Forgejo reads as an identity: `sudo` lets an administrator's
/// token act as another user, and the other two present a credential of the
/// pod's own beside the one this proxy sends.
const WITHHELD_PARAMETERS: [&str; 3] = ["sudo", "token", "access_token"];

/// The first [`WITHHELD_PARAMETERS`] key in the query, compared after
/// percent-decoding as Forgejo decodes it, so `%73udo` is `sudo`.
fn withheld_parameter(path_and_query: &str) -> Option<&'static str> {
    let query = path_and_query.split_once('?')?.1;
    // Fails closed: a query this cannot read is one Forgejo might.
    let Ok(url) = reqwest::Url::parse(&format!("http://forgejo.invalid/?{query}")) else {
        return Some("unreadable");
    };
    url.query_pairs().find_map(|(key, _)| {
        WITHHELD_PARAMETERS
            .into_iter()
            .find(|withheld| key.trim().eq_ignore_ascii_case(withheld))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_is_a_bare_dns_name_and_nothing_else() {
        assert_eq!(host("Git.Example.com").unwrap(), "git.example.com");
        for raw in [
            "git.example.com:3000",
            "user@git.example.com",
            "localhost",
            "git..example.com",
            "-git.example.com",
            "git.example.com/",
            "",
        ] {
            assert!(host(raw).is_err(), "{raw}");
        }
    }

    #[test]
    fn only_the_api_outside_admin_and_without_an_identity_parameter_is_forwarded() {
        for ok in [
            "/api/v1/repos/o/r",
            "/api/v1/repos/o/r/branch_protections/main",
            "/api/v1/orgs/o/repos?limit=50&page=2",
            "/api/v1/repos/o/r/contents/admin/file.yaml",
        ] {
            assert!(checked(ok).is_ok(), "{ok}");
        }
        for refused in [
            "/o/r.git/info/refs",
            "/api/v2/repos/o/r",
            "/api/v1/admin/users",
            "/api/v1/ADMIN/users",
            "/api/v1/admin",
            "/api/v1/repos/../admin/users",
            "/api/v1/%61dmin/users",
            "/api/v1/user?sudo=root",
            "/api/v1/user?limit=1&SUDO=root",
            "/api/v1/user?%73udo=root",
            "/api/v1/user?s%75do=root",
            "/api/v1/user?token=abc",
            "/api/v1/user?access%5Ftoken=abc",
        ] {
            assert!(checked(refused).is_err(), "{refused}");
        }
    }
}
