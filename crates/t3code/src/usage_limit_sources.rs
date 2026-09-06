//! Quota from a `CLIProxyAPI` hub, published on the `subscribeServerConfig`
//! stream as `usageLimitSourcesUpdated`.
//!
//! T3's own hub protocol, `GET /v0/management/quota-scheduler/status`, is a
//! convention third-party daemons implement for it. No `CLIProxyAPI` release or
//! plugin serves that path, so this backend reads the hub's real
//! `GET /v0/management/auth-files` and maps each pooled account itself.
//!
//! The hub reports quota as the upstream rate-limit headers it last saw on a
//! response for that account, so a window appears only after the account has
//! served a request through the hub.

use std::collections::BTreeMap;
use std::env;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, SecondsFormat, TimeZone, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{RwLock, broadcast};
use tracing::{debug, warn};

/// Read cadence. Matches the client's own provider health-check default, so
/// the hub rows age at the same rate as the local provider rows beside them.
const REFRESH_INTERVAL: Duration = Duration::from_mins(1);
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
const AUTH_FILES_PATH: &str = "/v0/management/auth-files";

/// The `id` of the one source this backend publishes. A T3 environment keys
/// its rows by source id, and the chart configures exactly one hub.
const SOURCE_ID: &str = "cliproxy";

const SESSION_WINDOW_MINS: u64 = 5 * 60;
const WEEK_WINDOW_MINS: u64 = 7 * 24 * 60;

/// Where the hub is and how to read it. Absent when the deployment configures
/// no hub, which is the condition under which the `usageLimitSources`
/// capability is withheld.
#[derive(Clone, Debug)]
struct HubConfig {
    url: String,
    management_key: String,
    label: String,
}

impl HubConfig {
    fn from_env() -> Option<Self> {
        let url = non_empty(env::var("T3_UI_RPC_CLIPROXY_URL").ok())?;
        let Some(management_key) = non_empty(env::var("T3_UI_RPC_CLIPROXY_MANAGEMENT_KEY").ok())
        else {
            // Without a key every management route answers 404, so a URL on its
            // own would publish a source that can only ever report an error.
            warn!(
                "T3_UI_RPC_CLIPROXY_URL is set without T3_UI_RPC_CLIPROXY_MANAGEMENT_KEY; \
                 no usage limit source is published"
            );
            return None;
        };
        let label = non_empty(env::var("T3_UI_RPC_CLIPROXY_LABEL").ok())
            .unwrap_or_else(|| "CLIProxyAPI".to_string());
        Some(Self {
            url: url.trim_end_matches('/').to_string(),
            management_key,
            label,
        })
    }
}

fn non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// The published snapshot and the fan-out that tells every open config stream
/// when it changes.
#[derive(Clone, Debug)]
pub struct UsageLimitSources {
    hub: Option<Arc<HubConfig>>,
    current: Arc<RwLock<Value>>,
    changes: broadcast::Sender<Value>,
}

impl UsageLimitSources {
    /// Reads the hub out of the environment and starts the poller. A
    /// deployment with no hub configured gets an inert value: no task, no
    /// capability, and no event.
    pub fn from_env(client: reqwest::Client) -> Self {
        let hub = HubConfig::from_env().map(Arc::new);
        let (changes, _) = broadcast::channel(8);
        let sources = Self {
            hub: hub.clone(),
            current: Arc::new(RwLock::new(json!([]))),
            changes,
        };
        if let Some(hub) = hub {
            let poller = sources.clone();
            tokio::spawn(async move { poller.poll_forever(client, hub).await });
        }
        sources
    }

    /// A value with no hub, for a caller that builds the state without reading
    /// the environment.
    pub fn disabled() -> Self {
        let (changes, _) = broadcast::channel(1);
        Self {
            hub: None,
            current: Arc::new(RwLock::new(json!([]))),
            changes,
        }
    }

    /// A value that publishes `sources` and never changes.
    #[cfg(test)]
    pub(crate) fn fixed(sources: Value) -> Self {
        let (changes, _) = broadcast::channel(1);
        Self {
            hub: None,
            current: Arc::new(RwLock::new(sources)),
            changes,
        }
    }

    /// Whether a hub is configured, and therefore whether this deployment
    /// reports `capabilities.usageLimitSources`. The client drops the event
    /// unless the snapshot declared the capability.
    pub fn configured(&self) -> bool {
        self.hub.is_some()
    }

    /// The current set, as the `usageLimitSourcesUpdated` payload's `sources`.
    pub async fn snapshot(&self) -> Value {
        self.current.read().await.clone()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Value> {
        self.changes.subscribe()
    }

    async fn poll_forever(self, client: reqwest::Client, hub: Arc<HubConfig>) {
        let mut ticker = tokio::time::interval(REFRESH_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            let next = read_hub(&client, &hub).await;
            let mut current = self.current.write().await;
            if *current == next {
                continue;
            }
            *current = next.clone();
            drop(current);
            // No subscriber is the steady state — nobody has the Limits view
            // open — so a closed channel is not worth a log line.
            let _ = self.changes.send(next);
        }
    }
}

/// One entry of the hub's `GET /v0/management/auth-files` answer. Only the
/// fields a usage row needs; the hub sends around twenty more.
#[derive(Debug, Deserialize)]
struct AuthFile {
    name: Option<String>,
    provider: Option<String>,
    email: Option<String>,
    #[serde(default)]
    disabled: bool,
    #[serde(default)]
    quota: QuotaObservation,
}

/// The hub's passive quota snapshot: the upstream rate-limit headers it last
/// saw for this account, keyed by canonical header name.
#[derive(Debug, Default, Deserialize)]
struct QuotaObservation {
    observed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    signals: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct AuthFilesBody {
    #[serde(default)]
    files: Vec<AuthFile>,
}

async fn read_hub(client: &reqwest::Client, hub: &HubConfig) -> Value {
    let checked_at = Utc::now();
    match fetch_auth_files(client, hub).await {
        Ok(body) => json!([source_row(
            hub,
            checked_at,
            accounts(&body.files, checked_at),
            None
        )]),
        Err(message) => {
            // The row stays with `error` set rather than vanishing, so the user
            // sees a hub that is configured and failing.
            debug!(hub = %hub.url, %message, "T3 usage limit source read failed");
            json!([source_row(hub, checked_at, Vec::new(), Some(message))])
        }
    }
}

fn source_row(
    hub: &HubConfig,
    checked_at: DateTime<Utc>,
    accounts: Vec<Value>,
    error: Option<String>,
) -> Value {
    let mut row = json!({
        "id": SOURCE_ID,
        "kind": "cliproxy",
        "label": hub.label,
        "checkedAt": iso(checked_at),
        "accounts": accounts,
    });
    if let Some(error) = error {
        row["error"] = json!(error);
    }
    row
}

async fn fetch_auth_files(
    client: &reqwest::Client,
    hub: &HubConfig,
) -> Result<AuthFilesBody, String> {
    let response = client
        .get(format!("{}{AUTH_FILES_PATH}", hub.url))
        .bearer_auth(&hub.management_key)
        .timeout(FETCH_TIMEOUT)
        .send()
        .await
        .map_err(|error| {
            if error.is_timeout() {
                "The hub did not answer in time.".to_string()
            } else {
                "The hub could not be reached.".to_string()
            }
        })?;
    let status = response.status();
    if !status.is_success() {
        // A management key that does not match answers 401, and a hub started
        // without one answers 404 on every management route.
        return Err(format!("The hub refused the request (HTTP {status})."));
    }
    let body = response
        .bytes()
        .await
        .map_err(|_| "The hub's answer could not be read.".to_string())?;
    serde_json::from_slice(&body)
        .map_err(|_| "The hub answered with an unexpected shape.".to_string())
}

fn accounts(files: &[AuthFile], checked_at: DateTime<Utc>) -> Vec<Value> {
    let mut accounts: Vec<Value> = files
        .iter()
        .filter(|file| !file.disabled)
        .filter_map(|file| account(file, checked_at))
        .collect();
    accounts.sort_by(|left, right| {
        let key = |value: &Value| {
            (
                value["driver"].as_str().unwrap_or_default().to_string(),
                value["id"].as_str().unwrap_or_default().to_string(),
            )
        };
        key(left).cmp(&key(right))
    });
    accounts
}

fn account(file: &AuthFile, checked_at: DateTime<Utc>) -> Option<Value> {
    let id = file
        .name
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())?;
    let provider = file.provider.as_deref().unwrap_or_default().to_lowercase();
    let driver = match provider.as_str() {
        "claude" => "claudeAgent",
        "codex" => "codex",
        // Every other pooled provider reports no quota headers at all, so a row
        // for one would be a heading with nothing under it.
        _ => return None,
    };
    let windows = match provider.as_str() {
        "claude" => claude_windows(&file.quota.signals),
        _ => codex_windows(&file.quota.signals),
    };
    let mut account = json!({
        "id": id,
        "driver": driver,
        "usageLimits": {
            "checkedAt": iso(file.quota.observed_at.unwrap_or(checked_at)),
            "windows": windows,
        },
    });
    if let Some(email) = non_empty(file.email.clone()) {
        account["email"] = json!(email);
    }
    if let Some(plan) = plan_label(&provider, &file.quota.signals) {
        account["plan"] = json!(plan);
    }
    Some(account)
}

/// Codex names its plan on every response; Claude reports no tier at all, so
/// the row says what it is rather than guessing a plan.
fn plan_label(provider: &str, signals: &BTreeMap<String, String>) -> Option<String> {
    match provider {
        "claude" => Some("Claude Subscription".to_string()),
        "codex" => {
            signal(signals, "X-Codex-Plan-Type").map(|plan| match plan.to_lowercase().as_str() {
                "plus" => "ChatGPT Plus Subscription".to_string(),
                "pro" => "ChatGPT Pro 20x Subscription".to_string(),
                "team" => "ChatGPT Team Subscription".to_string(),
                "enterprise" => "ChatGPT Enterprise Subscription".to_string(),
                _ => format!("ChatGPT {plan} Subscription"),
            })
        }
        _ => None,
    }
}

/// Header lookup is case-insensitive: the hub stores the canonical form Go
/// produced, and a proxy in front of it may have sent another casing.
fn signal<'a>(signals: &'a BTreeMap<String, String>, name: &str) -> Option<&'a str> {
    signals
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.trim())
        .filter(|value| !value.is_empty())
}

/// Codex sends a used percent per window, so both windows map directly.
/// `primary` is the rolling session and `secondary` the weekly allowance.
fn codex_windows(signals: &BTreeMap<String, String>) -> Vec<Value> {
    let limit_reached = signal(signals, "X-Codex-Limit-Reached")
        .is_some_and(|value| value.eq_ignore_ascii_case("true"));
    [
        ("primary", "Session", "session", SESSION_WINDOW_MINS),
        ("secondary", "Weekly", "weekly", WEEK_WINDOW_MINS),
    ]
    .into_iter()
    .filter_map(|(name, label, kind, default_mins)| {
        let used = signal(
            signals,
            &format!("X-Codex-{}-Used-Percent", capitalize(name)),
        )?
        .parse::<f64>()
        .ok()?;
        let mins = signal(
            signals,
            &format!("X-Codex-{}-Window-Minutes", capitalize(name)),
        )
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(default_mins);
        let resets_at = codex_reset(signals, name);
        Some(window(
            name,
            kind,
            label,
            if limit_reached { 100.0 } else { used },
            mins,
            resets_at,
        ))
    })
    .collect()
}

fn codex_reset(signals: &BTreeMap<String, String>, name: &str) -> Option<DateTime<Utc>> {
    let prefix = format!("X-Codex-{}", capitalize(name));
    if let Some(at) = signal(signals, &format!("{prefix}-Reset-At"))
        .and_then(|value| value.parse::<i64>().ok())
        .and_then(|seconds| Utc.timestamp_opt(seconds, 0).single())
    {
        return Some(at);
    }
    signal(signals, &format!("{prefix}-Reset-After-Seconds"))
        .and_then(|value| value.parse::<i64>().ok())
        .map(|seconds| Utc::now() + chrono::Duration::seconds(seconds))
}

/// Anthropic reports a status word per window and no percent, so a window
/// becomes a row only when the status is `rejected` — the one state that
/// states a number. `allowed` and `allowed_warning` carry no figure a bar
/// could draw, and a guessed one would read as measured.
fn claude_windows(signals: &BTreeMap<String, String>) -> Vec<Value> {
    [
        ("five_hour", "5h", "Session", "session", SESSION_WINDOW_MINS),
        ("seven_day", "7d", "Weekly", "weekly", WEEK_WINDOW_MINS),
        (
            "seven_day_fable",
            "7d_oi",
            "Weekly · Fable",
            "weekly",
            WEEK_WINDOW_MINS,
        ),
    ]
    .into_iter()
    .filter_map(|(id, header, label, kind, mins)| {
        let status = signal(
            signals,
            &format!("Anthropic-Ratelimit-Unified-{header}-Status"),
        )?;
        if !status.eq_ignore_ascii_case("rejected") {
            return None;
        }
        let resets_at = signal(
            signals,
            &format!("Anthropic-Ratelimit-Unified-{header}-Reset"),
        )
        .and_then(|value| value.parse::<i64>().ok())
        .and_then(|seconds| Utc.timestamp_opt(seconds, 0).single());
        Some(window(id, kind, label, 100.0, mins, resets_at))
    })
    .collect()
}

fn window(
    id: &str,
    kind: &str,
    label: &str,
    used_percent: f64,
    window_duration_mins: u64,
    resets_at: Option<DateTime<Utc>>,
) -> Value {
    let mut window = json!({
        "id": id,
        "kind": kind,
        "label": label,
        "usedPercent": used_percent.clamp(0.0, 100.0),
        "windowDurationMins": window_duration_mins,
    });
    if let Some(resets_at) = resets_at {
        window["resetsAt"] = json!(iso(resets_at));
    }
    window
}

fn capitalize(name: &str) -> String {
    let mut chars = name.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().collect::<String>() + chars.as_str()
    })
}

fn iso(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signals(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    fn file(provider: &str, name: &str, signals: BTreeMap<String, String>) -> AuthFile {
        AuthFile {
            name: Some(name.to_string()),
            provider: Some(provider.to_string()),
            email: Some("julius@ping.gg".to_string()),
            disabled: false,
            quota: QuotaObservation {
                observed_at: Utc.timestamp_opt(1_764_000_000, 0).single(),
                signals,
            },
        }
    }

    fn checked_at() -> DateTime<Utc> {
        Utc.timestamp_opt(1_764_003_600, 0)
            .single()
            .expect("a fixed timestamp is valid")
    }

    #[test]
    fn codex_windows_carry_the_percent_the_hub_observed() {
        let account = account(
            &file(
                "codex",
                "codex-7f42123a-julius@ping.gg-pro.json",
                signals(&[
                    ("X-Codex-Plan-Type", "pro"),
                    ("X-Codex-Primary-Used-Percent", "42.5"),
                    ("X-Codex-Primary-Window-Minutes", "300"),
                    ("X-Codex-Primary-Reset-At", "1764010800"),
                    ("X-Codex-Secondary-Used-Percent", "12"),
                    ("X-Codex-Secondary-Window-Minutes", "10080"),
                ]),
            ),
            checked_at(),
        )
        .expect("a codex account maps to a row");

        assert_eq!(account["driver"], "codex");
        assert_eq!(account["email"], "julius@ping.gg");
        assert_eq!(account["plan"], "ChatGPT Pro 20x Subscription");
        assert_eq!(
            account["usageLimits"]["windows"],
            json!([
                {
                    "id": "primary",
                    "kind": "session",
                    "label": "Session",
                    "usedPercent": 42.5,
                    "windowDurationMins": 300,
                    "resetsAt": "2025-11-24T19:00:00.000Z",
                },
                {
                    "id": "secondary",
                    "kind": "weekly",
                    "label": "Weekly",
                    "usedPercent": 12.0,
                    "windowDurationMins": 10080,
                },
            ])
        );
    }

    #[test]
    fn a_codex_limit_reached_header_pins_every_window_full() {
        let windows = codex_windows(&signals(&[
            ("X-Codex-Limit-Reached", "true"),
            ("X-Codex-Primary-Used-Percent", "88"),
        ]));

        assert_eq!(windows[0]["usedPercent"], 100.0);
    }

    #[test]
    fn a_rejected_claude_window_is_the_only_one_that_reports_a_percent() {
        let windows = claude_windows(&signals(&[
            ("Anthropic-Ratelimit-Unified-5h-Status", "allowed"),
            ("Anthropic-Ratelimit-Unified-7d-Status", "rejected"),
            ("Anthropic-Ratelimit-Unified-7d-Reset", "1764010800"),
        ]));

        assert_eq!(
            windows,
            vec![json!({
                "id": "seven_day",
                "kind": "weekly",
                "label": "Weekly",
                "usedPercent": 100.0,
                "windowDurationMins": 10080,
                "resetsAt": "2025-11-24T19:00:00.000Z",
            })]
        );
    }

    #[test]
    fn an_account_the_hub_has_never_seen_a_response_for_reports_no_windows() {
        let account = account(
            &file("claude", "claude-julius@ping.gg.json", signals(&[])),
            checked_at(),
        )
        .expect("a claude account maps to a row");

        assert_eq!(account["usageLimits"]["windows"], json!([]));
        assert_eq!(account["plan"], "Claude Subscription");
    }

    #[test]
    fn a_provider_that_reports_no_quota_headers_gets_no_row() {
        assert!(
            account(
                &file("gemini", "gemini-julius@ping.gg.json", signals(&[])),
                checked_at()
            )
            .is_none()
        );
        assert!(
            accounts(
                &[AuthFile {
                    disabled: true,
                    ..file("codex", "codex-julius@ping.gg.json", signals(&[]))
                }],
                checked_at()
            )
            .is_empty()
        );
    }

    #[test]
    fn accounts_sort_by_driver_then_file_name() {
        let rows = accounts(
            &[
                file("codex", "codex-b.json", signals(&[])),
                file("claude", "claude-b.json", signals(&[])),
                file("claude", "claude-a.json", signals(&[])),
            ],
            checked_at(),
        );

        let ids: Vec<&str> = rows.iter().filter_map(|row| row["id"].as_str()).collect();
        assert_eq!(ids, vec!["claude-a.json", "claude-b.json", "codex-b.json"]);
    }

    #[test]
    fn a_failed_read_keeps_the_row_and_names_the_failure() {
        let hub = HubConfig {
            url: "http://hub:8317".to_string(),
            management_key: "key".to_string(),
            label: "CLIProxyAPI".to_string(),
        };

        let row = source_row(&hub, checked_at(), Vec::new(), Some("nope".to_string()));

        assert_eq!(row["id"], SOURCE_ID);
        assert_eq!(row["label"], "CLIProxyAPI");
        assert_eq!(row["accounts"], json!([]));
        assert_eq!(row["error"], "nope");
    }
}
