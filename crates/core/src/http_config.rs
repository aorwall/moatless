//! The timeouts each HTTP integration's client is built with.
//!
//! Every bound reads `<PREFIX>_HTTP_CONNECT_TIMEOUT_SECS` and
//! `<PREFIX>_HTTP_TIMEOUT_SECS` (`_IDLE_TIMEOUT_SECS` for a stream), with the
//! prefix each field names.

use std::time::Duration;

use moatless_types::http::{HttpStreamTimeouts, HttpTimeouts};

/// HTTP client timeouts, one entry per integration whose bounds can differ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpConfig {
    /// GitHub's REST API: token and repository checks, the PR adapter, GitHub
    /// sign-in and App installation. Prefix `GITHUB`.
    pub github: HttpTimeouts,
    /// A self-hosted git host (Forgejo): access checks, token
    /// validation and instance calls. Task creation and start await the access
    /// check, so the total bounds a host that accepts and then stalls. Prefix
    /// `SELF_HOSTED_GIT`.
    pub self_hosted_git: HttpTimeouts,
    /// Skill, loop and repository-config file reads from any git host. Prefix
    /// `GIT_CONTENT`.
    pub git_content: HttpTimeouts,
    /// Plugin archive downloads, each a whole repository. Prefix
    /// `PLUGIN_ARCHIVE`.
    pub plugin_archive: HttpTimeouts,
    /// The git proxy's upstream, whose pack streams for as long as it moves.
    /// Prefix `GIT_PROXY`.
    pub git_proxy: HttpStreamTimeouts,
    /// The GitHub API proxy's upstream, which streams a log or release asset
    /// for as long as it moves. Prefix `GITHUB_PROXY`.
    pub github_proxy: HttpStreamTimeouts,
    /// The model gateway's upstream, an SSE stream that can sit quiet while
    /// the model thinks. Prefix `MODEL_GATEWAY`.
    pub model_gateway: HttpStreamTimeouts,
    /// Linear's GraphQL and OAuth APIs. Prefix `LINEAR`.
    pub linear: HttpTimeouts,
    /// Slack's Web API. Prefix `SLACK`.
    pub slack: HttpTimeouts,
    /// OIDC and Auth0 sign-in: discovery, token exchange and user info. Prefix
    /// `IDENTITY_PROVIDER`.
    pub identity_provider: HttpTimeouts,
    /// The decision model provider. Prefix `DECISION`.
    pub decision: HttpTimeouts,
    /// Codex token refresh and device login, and the branch listing sharing
    /// that client. Prefix `CODEX`.
    pub codex: HttpTimeouts,
    /// T3 client trace batches forwarded to the OTLP collector. Prefix
    /// `CLIENT_TRACES`.
    pub client_traces: HttpTimeouts,
    /// The usage-limit hub the T3 poller reads. Prefix `USAGE_LIMITS`.
    pub usage_limits: HttpTimeouts,
}

impl Default for HttpConfig {
    fn default() -> Self {
        let api = timeouts(10, 30);
        Self {
            github: api,
            self_hosted_git: api,
            git_content: api,
            plugin_archive: timeouts(10, 120),
            git_proxy: stream_timeouts(10, 300),
            github_proxy: stream_timeouts(10, 300),
            model_gateway: stream_timeouts(10, 600),
            linear: api,
            slack: api,
            identity_provider: api,
            decision: api,
            codex: timeouts(10, 15),
            client_traces: timeouts(5, 5),
            usage_limits: timeouts(10, 10),
        }
    }
}

impl HttpConfig {
    /// The defaults, each overridden by the variable `var` looks up.
    pub fn from_vars(var: impl Fn(&str) -> Option<String>) -> Self {
        let var = &var;
        let d = Self::default();
        Self {
            github: read(var, "GITHUB", d.github),
            self_hosted_git: read(var, "SELF_HOSTED_GIT", d.self_hosted_git),
            git_content: read(var, "GIT_CONTENT", d.git_content),
            plugin_archive: read(var, "PLUGIN_ARCHIVE", d.plugin_archive),
            git_proxy: read_stream(var, "GIT_PROXY", d.git_proxy),
            github_proxy: read_stream(var, "GITHUB_PROXY", d.github_proxy),
            model_gateway: read_stream(var, "MODEL_GATEWAY", d.model_gateway),
            linear: read(var, "LINEAR", d.linear),
            slack: read(var, "SLACK", d.slack),
            identity_provider: read(var, "IDENTITY_PROVIDER", d.identity_provider),
            decision: read(var, "DECISION", d.decision),
            codex: read(var, "CODEX", d.codex),
            client_traces: read(var, "CLIENT_TRACES", d.client_traces),
            usage_limits: read(var, "USAGE_LIMITS", d.usage_limits),
        }
    }
}

const fn timeouts(connect: u64, total: u64) -> HttpTimeouts {
    HttpTimeouts {
        connect: Duration::from_secs(connect),
        total: Duration::from_secs(total),
    }
}

const fn stream_timeouts(connect: u64, idle: u64) -> HttpStreamTimeouts {
    HttpStreamTimeouts {
        connect: Duration::from_secs(connect),
        idle: Duration::from_secs(idle),
    }
}

fn read_stream(
    var: &dyn Fn(&str) -> Option<String>,
    prefix: &str,
    default: HttpStreamTimeouts,
) -> HttpStreamTimeouts {
    HttpStreamTimeouts {
        connect: secs(
            var,
            &format!("{prefix}_HTTP_CONNECT_TIMEOUT_SECS"),
            default.connect,
        ),
        idle: secs(
            var,
            &format!("{prefix}_HTTP_IDLE_TIMEOUT_SECS"),
            default.idle,
        ),
    }
}

fn read(var: &dyn Fn(&str) -> Option<String>, prefix: &str, default: HttpTimeouts) -> HttpTimeouts {
    HttpTimeouts {
        connect: secs(
            var,
            &format!("{prefix}_HTTP_CONNECT_TIMEOUT_SECS"),
            default.connect,
        ),
        total: secs(var, &format!("{prefix}_HTTP_TIMEOUT_SECS"), default.total),
    }
}

/// `key` in whole seconds; zero or garbage keeps `default`.
fn secs(var: &dyn Fn(&str) -> Option<String>, key: &str, default: Duration) -> Duration {
    match var(key).and_then(|raw| raw.trim().parse::<u64>().ok()) {
        Some(secs) if secs > 0 => Duration::from_secs(secs),
        _ => default,
    }
}
