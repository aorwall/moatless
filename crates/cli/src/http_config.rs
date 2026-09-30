//! The timeouts each CLI HTTP client is built with.
//!
//! Every bound reads `MOAT_<PREFIX>_HTTP_CONNECT_TIMEOUT_SECS` and
//! `MOAT_<PREFIX>_HTTP_TIMEOUT_SECS` (`_IDLE_TIMEOUT_SECS` for a stream), with
//! the prefix each field names.

use std::time::Duration;

use moatless_types::http::{HttpStreamTimeouts, HttpTimeouts};

/// HTTP client timeouts, one entry per integration whose bounds can differ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpConfig {
    /// Every request to the Moatless API, and password login: long enough for
    /// a person at a terminal, short enough to notice a hang. `--timeout`
    /// raises the total for one invocation. Prefix `API`.
    pub api: HttpTimeouts,
    /// A streamed API response, bounded by its longest silence. The events
    /// stream pings every 30s; a followed log quiet for longer ends and has to
    /// be restarted. Prefix `API_STREAM`.
    pub api_stream: HttpStreamTimeouts,
    /// `moat login`'s probe for the server's login flow. Prefix `AUTH_PROBE`.
    pub auth_probe: HttpTimeouts,
    /// `moat docker status`'s health probes of the local stack. Prefix
    /// `LOCAL_STACK`.
    pub local_stack: HttpTimeouts,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            api: timeouts(10, 30),
            api_stream: HttpStreamTimeouts {
                connect: Duration::from_secs(10),
                idle: Duration::from_mins(30),
            },
            auth_probe: timeouts(5, 5),
            local_stack: timeouts(3, 3),
        }
    }
}

impl HttpConfig {
    /// The defaults, each overridden by the variable `var` looks up.
    pub fn from_vars(var: impl Fn(&str) -> Option<String>) -> Self {
        let var = &var;
        let d = Self::default();
        Self {
            api: read(var, "API", d.api),
            api_stream: HttpStreamTimeouts {
                connect: secs(
                    var,
                    "MOAT_API_STREAM_HTTP_CONNECT_TIMEOUT_SECS",
                    d.api_stream.connect,
                ),
                idle: secs(
                    var,
                    "MOAT_API_STREAM_HTTP_IDLE_TIMEOUT_SECS",
                    d.api_stream.idle,
                ),
            },
            auth_probe: read(var, "AUTH_PROBE", d.auth_probe),
            local_stack: read(var, "LOCAL_STACK", d.local_stack),
        }
    }
}

const fn timeouts(connect: u64, total: u64) -> HttpTimeouts {
    HttpTimeouts {
        connect: Duration::from_secs(connect),
        total: Duration::from_secs(total),
    }
}

fn read(var: &dyn Fn(&str) -> Option<String>, prefix: &str, default: HttpTimeouts) -> HttpTimeouts {
    HttpTimeouts {
        connect: secs(
            var,
            &format!("MOAT_{prefix}_HTTP_CONNECT_TIMEOUT_SECS"),
            default.connect,
        ),
        total: secs(
            var,
            &format!("MOAT_{prefix}_HTTP_TIMEOUT_SECS"),
            default.total,
        ),
    }
}

/// `key` in whole seconds; zero or garbage keeps `default`.
fn secs(var: &dyn Fn(&str) -> Option<String>, key: &str, default: Duration) -> Duration {
    match var(key).and_then(|raw| raw.trim().parse::<u64>().ok()) {
        Some(secs) if secs > 0 => Duration::from_secs(secs),
        _ => default,
    }
}
