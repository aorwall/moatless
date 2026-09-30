//! The timeouts each sandbox HTTP client is built with.
//!
//! Every bound reads `<PREFIX>_HTTP_CONNECT_TIMEOUT_SECS` and
//! `<PREFIX>_HTTP_TIMEOUT_SECS`, with the prefix each field names.

use std::time::Duration;

use moatless_types::http::HttpTimeouts;

/// HTTP client timeouts, one entry per integration whose bounds can differ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxHttpConfig {
    /// The `GET /user` lookup naming a GitHub token's login. Prefix `GITHUB`.
    pub github: HttpTimeouts,
    /// One clone-host reachability probe, HTTP or TCP, so a black-holed host
    /// cannot spend the whole retry budget inside a single connect. Prefix
    /// `GIT_PROBE`.
    pub git_probe: HttpTimeouts,
    /// One direct-to-S3 workspace PUT or GET: long enough for a large tar on a
    /// slow in-cluster link, bounded so a stuck transfer cannot wedge the
    /// upload path. Prefix `WORKSPACE_TRANSFER`.
    pub workspace_transfer: HttpTimeouts,
}

impl Default for SandboxHttpConfig {
    fn default() -> Self {
        Self {
            github: timeouts(Duration::from_secs(10), Duration::from_secs(10)),
            git_probe: timeouts(Duration::from_secs(5), Duration::from_secs(5)),
            workspace_transfer: timeouts(Duration::from_secs(10), Duration::from_mins(5)),
        }
    }
}

impl SandboxHttpConfig {
    /// The defaults, each overridden by the variable `var` looks up.
    pub fn from_vars(var: impl Fn(&str) -> Option<String>) -> Self {
        let var = &var;
        let d = Self::default();
        Self {
            github: read(var, "GITHUB", d.github),
            git_probe: read(var, "GIT_PROBE", d.git_probe),
            workspace_transfer: read(var, "WORKSPACE_TRANSFER", d.workspace_transfer),
        }
    }
}

const fn timeouts(connect: Duration, total: Duration) -> HttpTimeouts {
    HttpTimeouts { connect, total }
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
