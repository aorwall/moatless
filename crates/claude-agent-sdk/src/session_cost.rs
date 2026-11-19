//! A result's cost net of the Session total the CLI resumed with.
//!
//! Since Claude Code 2.1.280 a result's `total_cost_usd` and `modelUsage` are
//! the Session's running totals, restored on resume from the last `cost-state`
//! line in the transcript whose `sessionId` is the resumed id (a fork's cut
//! copy carries the last one before its anchor). The SDK reads that line
//! before spawning and subtracts it, so a result reports the run's own cost.
//! The CLI writes that line as it exits, after the result, so a resume first
//! waits for a CLI in this process still exiting on the Session.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use serde::Deserialize;
use tokio::sync::watch;
use tracing::warn;

use crate::api::content::ModelUsage;
use crate::api::messages::{ResultMessage, SdkMessage};
use crate::api::options::ClaudeOptions;
use crate::transcripts::{newest_transcript, resolve_project_dir};

/// Longer than `SubprocessTransport::close` takes to escalate to SIGKILL.
const EXIT_WAIT: Duration = Duration::from_secs(15);

/// Sessions whose CLI has ended its run but may not have exited yet, each with
/// a receiver that closes when it has. The `u64` tells two exits apart.
type Exiting = HashMap<String, (u64, watch::Receiver<()>)>;
static EXITING: LazyLock<Mutex<Exiting>> = LazyLock::new(Default::default);

/// Held from a run's end until its CLI has exited; dropping it releases a
/// resume waiting in [`wait_for_exits`].
#[derive(Debug)]
struct ExitGuard {
    session_id: String,
    id: u64,
    _tx: watch::Sender<()>,
}

impl ExitGuard {
    fn new(session_id: String) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (tx, rx) = watch::channel(());
        exiting().insert(session_id.clone(), (id, rx));
        Self {
            session_id,
            id,
            _tx: tx,
        }
    }
}

impl Drop for ExitGuard {
    fn drop(&mut self) {
        let mut map = exiting();
        if map
            .get(&self.session_id)
            .is_some_and(|(id, _)| *id == self.id)
        {
            map.remove(&self.session_id);
        }
    }
}

fn exiting() -> std::sync::MutexGuard<'static, Exiting> {
    EXITING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Wait for the CLI exiting on `session_id`, or on every Session when `None`.
async fn wait_for_exits(session_id: Option<&str>) {
    let pending: Vec<watch::Receiver<()>> = {
        let map = exiting();
        match session_id {
            Some(id) => map.get(id).map(|(_, rx)| rx.clone()).into_iter().collect(),
            None => map.values().map(|(_, rx)| rx.clone()).collect(),
        }
    };
    for mut rx in pending {
        let closed = async { while rx.changed().await.is_ok() {} };
        if tokio::time::timeout(EXIT_WAIT, closed).await.is_err() {
            warn!("a Claude CLI is still exiting; reading its Session's cost-state without it");
        }
    }
}

/// A Session's running totals, as a `cost-state` line or a result reports them,
/// and the orchestrator's hold on its Session until the CLI has exited.
#[derive(Debug, Default)]
pub(crate) struct SessionCost {
    total_cost_usd: f64,
    model_usage: HashMap<String, ModelUsage>,
    session_id: Option<String>,
    exiting: Option<ExitGuard>,
}

#[derive(Deserialize)]
struct CostStateLine {
    #[serde(rename = "type")]
    kind: String,
    #[serde(rename = "sessionId", default)]
    session_id: String,
    #[serde(rename = "totalCostUSD", default)]
    total_cost_usd: f64,
    #[serde(rename = "modelUsage", default)]
    model_usage: HashMap<String, ModelUsage>,
}

impl SessionCost {
    /// The totals the CLI will restore when spawned with `options`; zero for a
    /// fresh Session or a transcript that holds no `cost-state` line.
    pub(crate) async fn restored(options: &ClaudeOptions) -> Self {
        if options.resume.is_some() || options.continue_conversation == Some(true) {
            wait_for_exits(options.resume.as_deref()).await;
        }
        let Some((path, session_id)) = resumed_transcript(options).await else {
            return Self::default();
        };
        match tokio::fs::read_to_string(&path).await {
            Ok(body) => Self::last_cost_state(&body, &session_id),
            Err(error) => {
                warn!(%error, path = %path.display(), "could not read the resumed transcript; reporting the Session total as the run's cost");
                Self::default()
            }
        }
    }

    fn last_cost_state(transcript: &str, session_id: &str) -> Self {
        transcript
            .lines()
            .rev()
            .filter(|line| line.contains("\"cost-state\""))
            .filter_map(|line| {
                serde_json::from_str::<CostStateLine>(line)
                    .inspect_err(|error| warn!(%error, "unreadable cost-state line"))
                    .ok()
            })
            .find(|line| line.kind == "cost-state" && line.session_id == session_id)
            .map_or_default(|line| Self {
                total_cost_usd: line.total_cost_usd,
                model_usage: line.model_usage,
                ..Self::default()
            })
    }

    /// Net out a `Result`, and note the Session a `System` init names. A
    /// `Result` ends the run, so the Session is marked exiting before the
    /// caller can see it and resume.
    pub(crate) fn observe(&mut self, msg: &mut SdkMessage) {
        match msg {
            SdkMessage::System(init) => self.session_id = Some(init.session_id.clone()),
            SdkMessage::Result(result) => {
                let session_id = match result {
                    ResultMessage::Success(r) => r.session_id.clone(),
                    ResultMessage::Error(r) => r.session_id.clone(),
                };
                self.session_id = Some(session_id);
                self.mark_exiting();
                self.net_out(result);
            }
            _ => {}
        }
    }

    /// Hold the Session as exiting until `self` is dropped, which the
    /// orchestrator does after the CLI has exited.
    pub(crate) fn mark_exiting(&mut self) {
        if self.exiting.is_none() {
            self.exiting = self.session_id.clone().map(ExitGuard::new);
        }
    }

    /// Rewrite `result`'s totals to the increase over `self`, then take the
    /// reported totals as the baseline for a later result in the same process.
    ///
    /// A total below the baseline is left as reported: the counter restarted,
    /// so it holds this run alone.
    pub(crate) fn net_out(&mut self, result: &mut ResultMessage) {
        let (total_cost_usd, model_usage) = match result {
            ResultMessage::Success(r) => (&mut r.total_cost_usd, &mut r.model_usage),
            ResultMessage::Error(r) => (&mut r.total_cost_usd, &mut r.model_usage),
        };
        let reported_total = *total_cost_usd;
        let reported_usage = model_usage.clone();
        if reported_total + 1e-9 >= self.total_cost_usd {
            *total_cost_usd = (reported_total - self.total_cost_usd).max(0.0);
            for (model, usage) in model_usage.iter_mut() {
                if let Some(before) = self.model_usage.get(model) {
                    subtract(usage, before);
                }
            }
        }
        self.total_cost_usd = reported_total;
        self.model_usage = reported_usage;
    }
}

/// `ModelUsage` fields that accumulate over a Session; `context_window` and
/// `max_output_tokens` describe the model and are kept.
fn subtract(usage: &mut ModelUsage, before: &ModelUsage) {
    usage.input_tokens = usage.input_tokens.saturating_sub(before.input_tokens);
    usage.output_tokens = usage.output_tokens.saturating_sub(before.output_tokens);
    usage.cache_read_input_tokens = usage
        .cache_read_input_tokens
        .saturating_sub(before.cache_read_input_tokens);
    usage.cache_creation_input_tokens = usage
        .cache_creation_input_tokens
        .saturating_sub(before.cache_creation_input_tokens);
    usage.web_search_requests = usage
        .web_search_requests
        .saturating_sub(before.web_search_requests);
    usage.cost_usd = (usage.cost_usd - before.cost_usd).max(0.0);
}

/// The transcript `options` resumes and the Session id the CLI restores from:
/// the `--resume` id (also the source of `--fork-session`), or the newest
/// transcript under `--continue`.
async fn resumed_transcript(options: &ClaudeOptions) -> Option<(std::path::PathBuf, String)> {
    let dir = resolve_project_dir(options).ok()?;
    if let Some(id) = options.resume.as_ref() {
        return Some((dir.join(format!("{id}.jsonl")), id.clone()));
    }
    if options.continue_conversation == Some(true) {
        let path = newest_transcript(&dir).await?;
        let id = Path::new(&path).file_stem()?.to_str()?.to_string();
        return Some((path, id));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::content::Usage;
    use crate::api::messages::ResultSuccess;

    fn usage(output: u64, cost: f64) -> ModelUsage {
        ModelUsage {
            input_tokens: 0,
            output_tokens: output,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
            web_search_requests: 0,
            cost_usd: cost,
            context_window: 200_000,
            max_output_tokens: 0,
        }
    }

    fn result(total: f64, output: u64) -> ResultMessage {
        ResultMessage::Success(ResultSuccess {
            uuid: String::new(),
            session_id: "s1".into(),
            duration_ms: 0,
            duration_api_ms: 0,
            is_error: false,
            num_turns: 1,
            result: String::new(),
            total_cost_usd: total,
            usage: Usage {
                input_tokens: 0,
                output_tokens: output,
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
                service_tier: None,
                cache_creation: None,
                server_tool_use: None,
            },
            model_usage: HashMap::from([("opus".to_string(), usage(output, total))]),
            permission_denials: vec![],
            structured_output: None,
        })
    }

    fn totals(result: &ResultMessage) -> (f64, u64, f64, u64) {
        let ResultMessage::Success(r) = result else {
            unreachable!()
        };
        let m = &r.model_usage["opus"];
        (
            r.total_cost_usd,
            m.output_tokens,
            m.cost_usd,
            m.context_window,
        )
    }

    fn cost_state(session: &str, total: f64, output: u64) -> String {
        format!(
            r#"{{"type":"cost-state","sessionId":"{session}","totalCostUSD":{total},"modelUsage":{{"opus":{{"inputTokens":0,"outputTokens":{output},"thinkingTokens":0,"cacheReadInputTokens":0,"cacheCreationInputTokens":0,"webSearchRequests":0,"costUSD":{total}}}}}}}"#
        )
    }

    #[test]
    fn restores_the_last_cost_state_of_the_resumed_session() {
        let transcript = [
            r#"{"type":"user","sessionId":"s1"}"#.to_string(),
            cost_state("s1", 1.0, 100),
            r#"{"type":"assistant","sessionId":"s1"}"#.to_string(),
            cost_state("s1", 1.5, 200),
            // A line another Session wrote is not restored.
            cost_state("other", 9.0, 900),
        ]
        .join("\n");
        let restored = SessionCost::last_cost_state(&transcript, "s1");
        assert!((restored.total_cost_usd - 1.5).abs() < 1e-9);
        assert_eq!(restored.model_usage["opus"].output_tokens, 200);
    }

    #[test]
    fn a_transcript_without_cost_state_restores_zero() {
        let restored = SessionCost::last_cost_state(r#"{"type":"user","sessionId":"s1"}"#, "s1");
        assert!(restored.total_cost_usd.abs() < 1e-9);
        assert!(restored.model_usage.is_empty());
    }

    #[test]
    fn a_resumed_run_reports_its_increase() {
        let mut baseline = SessionCost::last_cost_state(&cost_state("s1", 1.5, 200), "s1");
        let mut run = result(2.2, 300);
        baseline.net_out(&mut run);
        let (total, output, cost, window) = totals(&run);
        assert!((total - 0.7).abs() < 1e-9);
        assert_eq!(output, 100);
        assert!((cost - 0.7).abs() < 1e-9);
        assert_eq!(window, 200_000);

        // A second result from the same process nets out the first.
        let mut next = result(2.5, 350);
        baseline.net_out(&mut next);
        assert!((totals(&next).0 - 0.3).abs() < 1e-9);
    }

    #[test]
    fn a_fresh_session_reports_its_total() {
        let mut run = result(1.0, 100);
        SessionCost::default().net_out(&mut run);
        assert!((totals(&run).0 - 1.0).abs() < 1e-9);
        assert_eq!(totals(&run).1, 100);
    }

    #[tokio::test]
    async fn a_resume_waits_for_the_session_to_exit() {
        let mut cost = SessionCost::default();
        let mut run = SdkMessage::Result(result(1.0, 100));
        cost.observe(&mut run);

        let waiter = tokio::spawn(async { wait_for_exits(Some("s1")).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!waiter.is_finished(), "s1's CLI is still exiting");
        // Another Session's resume does not wait.
        tokio::time::timeout(Duration::from_secs(1), wait_for_exits(Some("s2")))
            .await
            .expect("s2 has no exit pending");

        drop(cost);
        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("the exit released the resume")
            .unwrap();
        assert!(!exiting().contains_key("s1"));
    }

    #[test]
    fn a_total_below_the_baseline_is_the_runs_own() {
        let mut baseline = SessionCost::last_cost_state(&cost_state("s1", 1.5, 200), "s1");
        let mut run = result(0.4, 50);
        baseline.net_out(&mut run);
        assert!((totals(&run).0 - 0.4).abs() < 1e-9);
        assert_eq!(totals(&run).1, 50);
    }
}
