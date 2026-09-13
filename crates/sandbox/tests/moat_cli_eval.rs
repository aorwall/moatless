//! Live evaluation of how coding agents discover and use the `moat` CLI.
//!
//! The production Agent harness runs inside the pinned sandbox image. A safe,
//! scenario-driven `moat` shim is placed first on `PATH`: it records argv,
//! delegates help to the real CLI built from this checkout, and returns fixture
//! responses for explicitly allowed commands. It never contacts a backend.
//!
//! Run the deterministic tests with the rest of `moatless-sandbox`. Run the
//! credentialed evaluation through `task test:moat-cli:eval`.

#![allow(
    clippy::expect_used,
    clippy::ignore_without_reason,
    clippy::print_stderr,
    clippy::print_stdout,
    clippy::unwrap_used
)]

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use moatless_sandbox::agent::agent_state::AgentState;
use moatless_sandbox::agent::harness::{
    Cause, ClaudeConfig, CodexConfig, Harness, Input, Outcome, Rejection, Role, Signal,
};
use moatless_sandbox::config::{AgentConfig, Config, DEFAULT_COMMAND_TIMEOUT_SECS};
use moatless_sandbox::credentials::recovery::CredentialFailureDetector;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tempfile::TempDir;
use tokio::process::Command;

#[path = "moat_cli_eval/deterministic.rs"]
mod deterministic;
#[path = "moat_cli_eval/evaluation.rs"]
mod evaluation;

const EVAL_GATE: &str = "MOAT_CLI_EVAL";
const DEFAULT_REPETITIONS: usize = 3;
const DEFAULT_MIN_PASSES: usize = 2;
const CLAUDE_TIMEOUT: Duration = Duration::from_mins(2);
const CODEX_TIMEOUT: Duration = Duration::from_mins(3);
const MAX_TRAJECTORY_STRING_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
enum EvalAgent {
    ClaudeCode,
    Codex,
}

impl EvalAgent {
    const ALL: [Self; 2] = [Self::ClaudeCode, Self::Codex];

    fn parse(value: &str) -> Result<Self, String> {
        match value.trim() {
            "claude-code" | "claude" => Ok(Self::ClaudeCode),
            "codex" => Ok(Self::Codex),
            other => Err(format!(
                "unknown evaluation agent `{other}`; expected claude-code or codex"
            )),
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::Codex => "codex",
        }
    }

    fn cli_binary(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude",
            Self::Codex => "codex",
        }
    }

    fn timeout(self) -> Duration {
        match self {
            Self::ClaudeCode => CLAUDE_TIMEOUT,
            Self::Codex => CODEX_TIMEOUT,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Scenario {
    id: String,
    prompt: String,
    system_instruction: String,
    expectations: Vec<CommandExpectation>,
    mock_responses: BTreeMap<String, MockResponse>,
    #[serde(skip)]
    source_path: PathBuf,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CommandExpectation {
    command: Vec<String>,
    #[serde(default = "one")]
    min_invocations: usize,
    #[serde(default)]
    required_args: Vec<String>,
    #[serde(default)]
    forbidden_args: Vec<String>,
}

const fn one() -> usize {
    1
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MockResponse {
    text: String,
    json: Value,
    #[serde(default)]
    exit_code: i32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Invocation {
    argv: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct EvalReport {
    commit: String,
    generated_at: String,
    settings: EvalSettingsReport,
    agent_cli_versions: BTreeMap<String, String>,
    trials: Vec<TrialReport>,
    summaries: Vec<AgentSummary>,
    passed: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct EvalSettingsReport {
    repetitions: usize,
    min_passes: usize,
    scenarios: Vec<String>,
    agents: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TrialReport {
    scenario_id: String,
    prompt: String,
    system_instruction: String,
    agent: EvalAgent,
    repetition: usize,
    requested_model: Option<String>,
    resolved_model: Option<String>,
    duration_ms: u64,
    terminal_outcome: String,
    trajectory: Vec<TrajectoryEvent>,
    invocations: Vec<Invocation>,
    expectation_counts: BTreeMap<String, usize>,
    passed: bool,
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
enum TrajectoryEvent {
    HarnessSubmitAccepted {
        elapsed_ms: u64,
    },
    ExecutionStarted {
        elapsed_ms: u64,
        session_id: String,
        cause: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        input_id: Option<String>,
    },
    InputAccepted {
        elapsed_ms: u64,
        input_id: String,
    },
    Source {
        elapsed_ms: u64,
        source_id: String,
        message_id: String,
        session_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        parent_tool_use_id: Option<String>,
        role: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        block: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        tool_use_result: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        usage: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        diagnostics: Option<Value>,
    },
    Activity {
        elapsed_ms: u64,
        session_id: String,
        detail: String,
    },
    ContextWindow {
        elapsed_ms: u64,
        used_tokens: i64,
        #[serde(skip_serializing_if = "Option::is_none")]
        max_tokens: Option<i64>,
    },
    ExecutionFinished {
        elapsed_ms: u64,
        outcome: String,
    },
    InputRejected {
        elapsed_ms: u64,
        input_id: String,
        reason: String,
    },
    SignalStreamEnded {
        elapsed_ms: u64,
    },
    SubmitFailed {
        elapsed_ms: u64,
        error: String,
    },
    TimedOut {
        elapsed_ms: u64,
        timeout_ms: u64,
    },
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentSummary {
    scenario_id: String,
    agent: EvalAgent,
    passes: usize,
    trials: usize,
    required_passes: usize,
    passed: bool,
}

#[derive(Debug)]
struct EvalSettings {
    repetitions: usize,
    min_passes: usize,
    agents: Vec<EvalAgent>,
    scenario_filter: Option<Vec<String>>,
    assets_dir: PathBuf,
    real_moat_bin: PathBuf,
    report_path: PathBuf,
}

#[derive(Debug)]
enum TrialTerminal {
    Completed,
    Cancelled,
    Failed(String),
    Rejected(String),
    TimedOut,
}

impl TrialTerminal {
    fn is_completed(&self) -> bool {
        matches!(self, Self::Completed)
    }

    fn label(&self) -> String {
        match self {
            Self::Completed => "completed".to_string(),
            Self::Cancelled => "cancelled".to_string(),
            Self::Failed(error) => format!("failed: {error}"),
            Self::Rejected(error) => format!("rejected: {error}"),
            Self::TimedOut => "timed_out".to_string(),
        }
    }
}

#[derive(Debug)]
struct ReportRedactor {
    secrets: Vec<String>,
}

impl ReportRedactor {
    fn from_environment() -> Self {
        let values = [
            "CLAUDE_CODE_OAUTH_TOKEN",
            "ANTHROPIC_API_KEY",
            "OPENAI_API_KEY",
            "CODEX_AUTH_JSON",
        ]
        .into_iter()
        .filter_map(|name| std::env::var(name).ok());
        Self::new(values)
    }

    fn new<I, S>(values: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut secrets = Vec::new();
        for value in values {
            let value = value.as_ref();
            push_secret(&mut secrets, value);
            if let Ok(json) = serde_json::from_str::<Value>(value) {
                collect_json_secrets(&json, false, &mut secrets);
            }
        }
        secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
        secrets.dedup();
        Self { secrets }
    }

    fn text(&self, value: &str) -> String {
        let mut redacted = value.to_string();
        for secret in &self.secrets {
            redacted = redacted.replace(secret, "<redacted>");
        }
        truncate_report_text(redacted)
    }

    fn value(&self, value: Value) -> Value {
        match value {
            Value::Array(values) => {
                Value::Array(values.into_iter().map(|value| self.value(value)).collect())
            }
            Value::Object(values) => Value::Object(
                values
                    .into_iter()
                    .map(|(key, value)| {
                        let value = if credential_name(&key) {
                            Value::String("<redacted>".to_string())
                        } else {
                            self.value(value)
                        };
                        (key, value)
                    })
                    .collect(),
            ),
            Value::String(value) => Value::String(self.text(&value)),
            other => other,
        }
    }
}

#[derive(Debug)]
struct TrajectoryRecorder {
    started: Instant,
    redactor: ReportRedactor,
    resolved_model: Option<String>,
    events: Vec<TrajectoryEvent>,
}

impl TrajectoryRecorder {
    fn new(started: Instant, redactor: ReportRedactor) -> Self {
        Self {
            started,
            redactor,
            resolved_model: None,
            events: Vec::new(),
        }
    }

    fn elapsed_ms(&self) -> u64 {
        elapsed_ms(self.started)
    }

    fn record_submit_accepted(&mut self) {
        self.events.push(TrajectoryEvent::HarnessSubmitAccepted {
            elapsed_ms: self.elapsed_ms(),
        });
    }

    fn record_submit_failed(&mut self, error: &str) {
        self.events.push(TrajectoryEvent::SubmitFailed {
            elapsed_ms: self.elapsed_ms(),
            error: self.redactor.text(error),
        });
    }

    fn record_timeout(&mut self, timeout: Duration) {
        self.events.push(TrajectoryEvent::TimedOut {
            elapsed_ms: self.elapsed_ms(),
            timeout_ms: duration_ms(timeout),
        });
    }

    fn observe(&mut self, signal: Signal) -> Option<TrialTerminal> {
        let elapsed_ms = self.elapsed_ms();
        match signal {
            Signal::ExecutionStarted { session, cause, .. } => {
                let (cause, input_id) = match cause {
                    Cause::Input(input) => ("input".to_string(), Some(input.0)),
                    Cause::Internal => ("internal".to_string(), None),
                };
                self.events.push(TrajectoryEvent::ExecutionStarted {
                    elapsed_ms,
                    session_id: session.0,
                    cause,
                    input_id,
                });
                None
            }
            Signal::InputAccepted { input } => {
                self.events.push(TrajectoryEvent::InputAccepted {
                    elapsed_ms,
                    input_id: input.0,
                });
                None
            }
            Signal::Source(source) => {
                if self.resolved_model.is_none() {
                    self.resolved_model.clone_from(&source.model);
                }
                let block = self.redactor.value(source.block_json());
                let role = match source.role {
                    Role::Assistant => "assistant",
                    Role::User => "user",
                    Role::System => "system",
                };
                self.events.push(TrajectoryEvent::Source {
                    elapsed_ms,
                    source_id: source.source_id.0,
                    message_id: source.message_id.0,
                    session_id: source.session.0,
                    parent_tool_use_id: source.parent_tool_use_id,
                    role: role.to_string(),
                    model: source.model,
                    block,
                    tool_use_result: source
                        .tool_use_result
                        .map(|value| self.redactor.value(value)),
                    usage: source.usage.map(|value| self.redactor.value(value)),
                    diagnostics: source.diagnostics.map(|value| self.redactor.value(value)),
                });
                None
            }
            Signal::Activity(activity) => {
                self.events.push(TrajectoryEvent::Activity {
                    elapsed_ms,
                    session_id: activity.session.0,
                    detail: self.redactor.text(&activity.detail),
                });
                None
            }
            Signal::ContextWindow(context) => {
                self.events.push(TrajectoryEvent::ContextWindow {
                    elapsed_ms,
                    used_tokens: context.used_tokens,
                    max_tokens: context.max_tokens,
                });
                None
            }
            Signal::ExecutionFinished { outcome } => {
                let (terminal, outcome) = match outcome {
                    Outcome::Completed { .. } => {
                        (TrialTerminal::Completed, "completed".to_string())
                    }
                    Outcome::Cancelled => (TrialTerminal::Cancelled, "cancelled".to_string()),
                    Outcome::Failed(error) => {
                        let error = self.redactor.text(&error.to_string());
                        (
                            TrialTerminal::Failed(error.clone()),
                            format!("failed: {error}"),
                        )
                    }
                };
                self.events.push(TrajectoryEvent::ExecutionFinished {
                    elapsed_ms,
                    outcome,
                });
                Some(terminal)
            }
            Signal::InputRejected { input, reason } => {
                let (terminal, reason) = match reason {
                    Rejection::Cancelled => (
                        TrialTerminal::Rejected(
                            "input was cancelled before the evaluation Turn started".to_string(),
                        ),
                        "cancelled".to_string(),
                    ),
                    Rejection::Failed(error) => {
                        let error = self.redactor.text(&error.to_string());
                        (
                            TrialTerminal::Rejected(error.clone()),
                            format!("failed: {error}"),
                        )
                    }
                };
                self.events.push(TrajectoryEvent::InputRejected {
                    elapsed_ms,
                    input_id: input.0,
                    reason,
                });
                Some(terminal)
            }
        }
    }

    fn record_signal_stream_ended(&mut self) {
        self.events.push(TrajectoryEvent::SignalStreamEnded {
            elapsed_ms: self.elapsed_ms(),
        });
    }
}

fn push_secret(secrets: &mut Vec<String>, value: &str) {
    if value.len() >= 8 {
        secrets.push(value.to_string());
    }
}

fn collect_json_secrets(value: &Value, inherited_sensitive: bool, secrets: &mut Vec<String>) {
    match value {
        Value::Object(values) => {
            for (key, value) in values {
                collect_json_secrets(value, inherited_sensitive || credential_name(key), secrets);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_json_secrets(value, inherited_sensitive, secrets);
            }
        }
        Value::String(value) if inherited_sensitive => push_secret(secrets, value),
        _ => {}
    }
}

fn credential_name(value: &str) -> bool {
    let normalized = value
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect::<String>();
    normalized == "token"
        || normalized.ends_with("token")
        || normalized.contains("password")
        || normalized.contains("secret")
        || normalized.contains("apikey")
        || normalized == "auth"
        || normalized == "authorization"
}

fn truncate_report_text(mut value: String) -> String {
    if value.len() <= MAX_TRAJECTORY_STRING_BYTES {
        return value;
    }
    let original_len = value.len();
    let mut boundary = MAX_TRAJECTORY_STRING_BYTES;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value.truncate(boundary);
    format!("{value}…<truncated {} bytes>", original_len - boundary)
}

fn duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

impl EvalSettings {
    fn from_env() -> Result<Self, String> {
        let repetitions = parse_positive_env("MOAT_CLI_EVAL_REPETITIONS", DEFAULT_REPETITIONS)?;
        let min_passes = parse_positive_env("MOAT_CLI_EVAL_MIN_PASSES", DEFAULT_MIN_PASSES)?;
        if min_passes > repetitions {
            return Err(format!(
                "MOAT_CLI_EVAL_MIN_PASSES ({min_passes}) exceeds repetitions ({repetitions})"
            ));
        }

        let agents = match std::env::var("MOAT_CLI_EVAL_AGENTS") {
            Ok(raw) => parse_csv(&raw)
                .into_iter()
                .map(|value| EvalAgent::parse(&value))
                .collect::<Result<Vec<_>, _>>()?,
            Err(_) => EvalAgent::ALL.to_vec(),
        };
        if agents.is_empty() {
            return Err("MOAT_CLI_EVAL_AGENTS selected no agents".to_string());
        }

        let scenario_filter = std::env::var("MOAT_CLI_EVAL_SCENARIOS")
            .ok()
            .map(|raw| parse_csv(&raw));
        if scenario_filter.as_ref().is_some_and(Vec::is_empty) {
            return Err("MOAT_CLI_EVAL_SCENARIOS selected no scenarios".to_string());
        }

        let assets_dir = std::env::var_os("MOAT_CLI_EVAL_ASSETS").map_or_else(
            || Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/moat_cli_eval"),
            PathBuf::from,
        );
        let real_moat_bin = std::env::var_os("MOAT_CLI_REAL_BIN")
            .map_or_else(|| PathBuf::from("/usr/local/bin/moat-real"), PathBuf::from);
        let report_path = std::env::var_os("MOAT_CLI_EVAL_REPORT").map_or_else(
            || {
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .parent()
                    .and_then(Path::parent)
                    .unwrap_or_else(|| Path::new("."))
                    .join("test-results/moat-cli-eval/results.json")
            },
            PathBuf::from,
        );

        let settings = Self {
            repetitions,
            min_passes,
            agents,
            scenario_filter,
            assets_dir,
            real_moat_bin,
            report_path,
        };
        settings.validate_prerequisites()?;
        Ok(settings)
    }

    fn validate_prerequisites(&self) -> Result<(), String> {
        let shim = self.assets_dir.join("mock_moat.sh");
        if !shim.is_file() {
            return Err(format!("mock moat shim is missing: {}", shim.display()));
        }
        if !self.real_moat_bin.is_file() {
            return Err(format!(
                "real moat CLI is missing: {}",
                self.real_moat_bin.display()
            ));
        }
        for agent in &self.agents {
            match agent {
                EvalAgent::ClaudeCode
                    if !has_any_env(&["CLAUDE_CODE_OAUTH_TOKEN", "ANTHROPIC_API_KEY"]) =>
                {
                    return Err(
                        "Claude evaluation requires CLAUDE_CODE_OAUTH_TOKEN or ANTHROPIC_API_KEY"
                            .to_string(),
                    );
                }
                EvalAgent::Codex if !has_any_env(&["OPENAI_API_KEY", "CODEX_AUTH_JSON"]) => {
                    return Err(
                        "Codex evaluation requires OPENAI_API_KEY or CODEX_AUTH_JSON".to_string(),
                    );
                }
                _ => {}
            }
        }
        Ok(())
    }
}

fn parse_positive_env(name: &str, default: usize) -> Result<usize, String> {
    let Ok(raw) = std::env::var(name) else {
        return Ok(default);
    };
    let value = raw
        .parse::<usize>()
        .map_err(|error| format!("{name} must be a positive integer: {error}"))?;
    if value == 0 {
        return Err(format!("{name} must be greater than zero"));
    }
    Ok(value)
}

fn parse_csv(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn has_any_env(names: &[&str]) -> bool {
    names
        .iter()
        .any(|name| std::env::var(name).is_ok_and(|value| !value.trim().is_empty()))
}

fn load_scenarios(settings: &EvalSettings) -> Result<Vec<Scenario>, String> {
    let scenarios_dir = settings.assets_dir.join("scenarios");
    let mut paths = fs::read_dir(&scenarios_dir)
        .map_err(|error| format!("read {}: {error}", scenarios_dir.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect::<Vec<_>>();
    paths.sort();

    let mut scenarios = Vec::new();
    for path in paths {
        let raw = fs::read_to_string(&path)
            .map_err(|error| format!("read {}: {error}", path.display()))?;
        let mut scenario: Scenario = serde_json::from_str(&raw)
            .map_err(|error| format!("parse {}: {error}", path.display()))?;
        scenario.source_path.clone_from(&path);
        validate_scenario(&scenario)?;
        if settings
            .scenario_filter
            .as_ref()
            .is_none_or(|filter| filter.contains(&scenario.id))
        {
            scenarios.push(scenario);
        }
    }

    if scenarios.is_empty() {
        return Err(format!(
            "no evaluation scenarios selected from {}",
            scenarios_dir.display()
        ));
    }
    if let Some(filter) = &settings.scenario_filter {
        let missing = filter
            .iter()
            .filter(|id| !scenarios.iter().any(|scenario| &scenario.id == *id))
            .cloned()
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(format!("unknown scenario ids: {}", missing.join(", ")));
        }
    }
    Ok(scenarios)
}

fn validate_scenario(scenario: &Scenario) -> Result<(), String> {
    if scenario.id.trim().is_empty() {
        return Err("scenario id must not be empty".to_string());
    }
    if scenario.prompt.trim().is_empty() {
        return Err(format!("{}: prompt must not be empty", scenario.id));
    }
    if scenario.system_instruction.trim().is_empty() {
        return Err(format!(
            "{}: systemInstruction must not be empty",
            scenario.id
        ));
    }
    if scenario.expectations.is_empty() {
        return Err(format!(
            "{}: at least one command expectation is required",
            scenario.id
        ));
    }
    for expectation in &scenario.expectations {
        if expectation.command.is_empty()
            || expectation
                .command
                .iter()
                .any(|part| part.trim().is_empty())
        {
            return Err(format!(
                "{}: expectation commands must contain non-empty tokens",
                scenario.id
            ));
        }
        if expectation.min_invocations == 0 {
            return Err(format!(
                "{}: minInvocations must be greater than zero",
                scenario.id
            ));
        }
        let key = expectation.command.join(" ");
        if !scenario.mock_responses.contains_key(&key) {
            return Err(format!(
                "{}: expected command `{key}` has no safe mock response",
                scenario.id
            ));
        }
    }
    for (key, response) in &scenario.mock_responses {
        if key.trim().is_empty() || response.text.is_empty() && response.json.is_null() {
            return Err(format!(
                "{}: mock response `{key}` must have text or JSON output",
                scenario.id
            ));
        }
        if !(0..=255).contains(&response.exit_code) {
            return Err(format!(
                "{}: mock response `{key}` has invalid exitCode {}",
                scenario.id, response.exit_code
            ));
        }
    }
    Ok(())
}

fn normalized_command(argv: &[String]) -> Vec<&str> {
    let mut command = Vec::new();
    let mut skip_next = false;
    for arg in argv {
        if skip_next {
            skip_next = false;
            continue;
        }
        if arg == "--timeout" {
            skip_next = true;
            continue;
        }
        if arg.starts_with("--timeout=") || matches!(arg.as_str(), "--json" | "--verbose") {
            continue;
        }
        if arg.starts_with('-') {
            continue;
        }
        command.push(arg.as_str());
        if command.len() == 2 {
            break;
        }
    }
    command
}

fn is_help_invocation(argv: &[String]) -> bool {
    argv.iter()
        .any(|arg| matches!(arg.as_str(), "-h" | "--help"))
        || normalized_command(argv).contains(&"help")
}

fn contains_arg(argv: &[String], expected: &str) -> bool {
    argv.iter().any(|arg| {
        arg == expected
            || expected.starts_with("--")
                && arg
                    .strip_prefix(expected)
                    .is_some_and(|suffix| suffix.starts_with('='))
    })
}

fn invocation_matches(expectation: &CommandExpectation, invocation: &Invocation) -> bool {
    if is_help_invocation(&invocation.argv) {
        return false;
    }
    let command = normalized_command(&invocation.argv);
    command
        == expectation
            .command
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
        && expectation
            .required_args
            .iter()
            .all(|arg| contains_arg(&invocation.argv, arg))
        && expectation
            .forbidden_args
            .iter()
            .all(|arg| !contains_arg(&invocation.argv, arg))
}

fn expectation_counts(scenario: &Scenario, invocations: &[Invocation]) -> BTreeMap<String, usize> {
    scenario
        .expectations
        .iter()
        .map(|expectation| {
            let count = invocations
                .iter()
                .filter(|invocation| invocation_matches(expectation, invocation))
                .count();
            (expectation.command.join(" "), count)
        })
        .collect()
}

fn expectations_pass(scenario: &Scenario, counts: &BTreeMap<String, usize>) -> bool {
    scenario.expectations.iter().all(|expectation| {
        counts
            .get(&expectation.command.join(" "))
            .is_some_and(|count| *count >= expectation.min_invocations)
    })
}

fn redact_invocations(invocations: Vec<Invocation>) -> Vec<Invocation> {
    invocations
        .into_iter()
        .map(|invocation| Invocation {
            argv: redact_argv(&invocation.argv),
        })
        .collect()
}

fn redact_argv(argv: &[String]) -> Vec<String> {
    let mut redacted = Vec::with_capacity(argv.len());
    let mut redact_next = false;
    for arg in argv {
        if redact_next {
            redacted.push("<redacted>".to_string());
            redact_next = false;
            continue;
        }
        if let Some((flag, _)) = arg.split_once('=') {
            if credential_flag(flag) {
                redacted.push(format!("{flag}=<redacted>"));
                continue;
            }
        }
        redacted.push(arg.clone());
        redact_next = credential_flag(arg);
    }
    redacted
}

fn credential_flag(arg: &str) -> bool {
    if !arg.starts_with('-') {
        return false;
    }
    let flag = arg.trim_start_matches('-').to_ascii_lowercase();
    flag.contains("token")
        || flag.contains("password")
        || flag.contains("secret")
        || flag.contains("api-key")
        || flag == "auth"
}

fn read_invocations(path: &Path) -> Result<Vec<Invocation>, String> {
    let Ok(raw) = fs::read_to_string(path) else {
        return Ok(Vec::new());
    };
    raw.lines()
        .enumerate()
        .map(|(index, line)| {
            serde_json::from_str(line).map_err(|error| {
                format!(
                    "parse command log {} line {}: {error}",
                    path.display(),
                    index + 1
                )
            })
        })
        .collect()
}

fn build_config(tmp: &TempDir, agent: EvalAgent) -> Config {
    let home = tmp.path().join("home");
    Config {
        port: 0,
        grpc_port: 0,
        sandbox_id: format!("moat-cli-eval-{}", agent.id()),
        task_id: format!("moat-cli-eval-{}", agent.id()),
        agent_type: Some(agent.id().to_string()),
        workspace_dir: tmp.path().join("workspace").display().to_string(),
        repo_dir: tmp.path().join("workspace").display().to_string(),
        data_dir: tmp.path().join("data").display().to_string(),
        runtime_dir: tmp.path().join("run").display().to_string(),
        upload_dir: tmp.path().join("uploads").display().to_string(),
        log_level: "info".to_string(),
        backend_grpc_url: "http://localhost:50051".to_string(),
        backend_api_url: None,
        user_auth_token: None,
        bootstrap_token: None,
        agent_config: AgentConfig::default(),
        claude_cli_path: std::env::var("SANDBOX_CLAUDE_EXECUTABLE_PATH")
            .unwrap_or_else(|_| "/usr/local/bin/claude".to_string()),
        codex_cli_path: std::env::var("SANDBOX_CODEX_EXECUTABLE_PATH")
            .unwrap_or_else(|_| "/usr/local/bin/codex".to_string()),
        servers: Vec::new(),
        frontend_url: None,
        default_model: None,
        default_codex_model: None,
        openai_base_url: None,
        use_tmux: false,
        command_default_timeout_secs: DEFAULT_COMMAND_TIMEOUT_SECS,
        workspace_restore_if_empty: false,
        home_dir: home.display().to_string(),
        codex_home_dir: home.join(".codex").display().to_string(),
        claude_config_dir: home.join(".claude").display().to_string(),
        git_clone_url: None,
        git_branch: None,
        git_provider: "github".to_string(),
        repos: Vec::new(),
        repo_config_raw: None,
        setup_commands: Vec::new(),
        plugins: Vec::new(),
        permission_mode: claude_agent_sdk::PermissionMode::BypassPermissions,
        workspace_sync_enabled: false,
        headless: true,
    }
}

fn prepare_trial(
    settings: &EvalSettings,
    scenario: &Scenario,
    agent: EvalAgent,
) -> Result<(TempDir, Config, PathBuf, HashMap<String, String>), String> {
    let tmp = TempDir::new().map_err(|error| format!("create trial directory: {error}"))?;
    for subdir in [
        "workspace",
        "data",
        "run",
        "uploads",
        "home/.claude",
        "home/.codex",
        "bin",
    ] {
        fs::create_dir_all(tmp.path().join(subdir))
            .map_err(|error| format!("create trial {subdir}: {error}"))?;
    }

    let mock_bin = tmp.path().join("bin/moat");
    fs::copy(settings.assets_dir.join("mock_moat.sh"), &mock_bin)
        .map_err(|error| format!("stage mock moat: {error}"))?;
    let mut permissions = fs::metadata(&mock_bin)
        .map_err(|error| format!("inspect mock moat: {error}"))?
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&mock_bin, permissions)
        .map_err(|error| format!("make mock moat executable: {error}"))?;

    let config = build_config(&tmp, agent);
    if agent == EvalAgent::Codex {
        if let Ok(auth_json) = std::env::var("CODEX_AUTH_JSON") {
            fs::write(
                Path::new(&config.codex_home_dir).join("auth.json"),
                auth_json,
            )
            .map_err(|error| format!("stage Codex auth: {error}"))?;
        }
    }

    let log_path = tmp.path().join("moat-invocations.jsonl");
    let inherited_path = std::env::var("PATH").unwrap_or_default();
    let mut env = HashMap::from([
        (
            "PATH".to_string(),
            format!("{}:{inherited_path}", tmp.path().join("bin").display()),
        ),
        ("HOME".to_string(), config.home_dir.clone()),
        (
            "CLAUDE_CONFIG_DIR".to_string(),
            config.claude_config_dir.clone(),
        ),
        ("CODEX_HOME".to_string(), config.codex_home_dir.clone()),
        ("IS_SANDBOX".to_string(), "1".to_string()),
        (
            "USER_AUTH_TOKEN".to_string(),
            "moat-cli-eval-no-backend".to_string(),
        ),
        ("TASK_ID".to_string(), config.task_id.clone()),
        ("AGENT_TYPE".to_string(), agent.id().to_string()),
        (
            "MOAT_CLI_REAL_BIN".to_string(),
            settings.real_moat_bin.display().to_string(),
        ),
        (
            "MOAT_CLI_EVAL_SCENARIO_FILE".to_string(),
            scenario.source_path.display().to_string(),
        ),
        (
            "MOAT_CLI_EVAL_LOG".to_string(),
            log_path.display().to_string(),
        ),
    ]);
    let credential_names: &[&str] = match agent {
        EvalAgent::ClaudeCode => &["CLAUDE_CODE_OAUTH_TOKEN", "ANTHROPIC_API_KEY"],
        EvalAgent::Codex => &["OPENAI_API_KEY"],
    };
    for name in credential_names {
        if let Ok(value) = std::env::var(name) {
            env.insert((*name).to_string(), value);
        }
    }

    Ok((tmp, config, log_path, env))
}

async fn run_trial(
    settings: &EvalSettings,
    scenario: &Scenario,
    agent: EvalAgent,
    repetition: usize,
) -> TrialReport {
    let started = Instant::now();
    let (tmp, config, log_path, env) = match prepare_trial(settings, scenario, agent) {
        Ok(prepared) => prepared,
        Err(error) => {
            return TrialReport {
                scenario_id: scenario.id.clone(),
                prompt: scenario.prompt.clone(),
                system_instruction: scenario.system_instruction.clone(),
                agent,
                repetition,
                requested_model: None,
                resolved_model: None,
                duration_ms: elapsed_ms(started),
                terminal_outcome: error,
                trajectory: Vec::new(),
                invocations: Vec::new(),
                expectation_counts: BTreeMap::new(),
                passed: false,
            };
        }
    };

    let mut harness = match agent {
        EvalAgent::ClaudeCode => Harness::claude(ClaudeConfig {
            config: Arc::new(config),
            agent_state: Arc::new(AgentState::new()),
            agent_type: Some(agent.id().to_string()),
            events: None,
        }),
        EvalAgent::Codex => Harness::codex(CodexConfig {
            config: Arc::new(config),
            credential_failures: CredentialFailureDetector::default(),
        }),
    };

    let mut input = Input::text(
        format!("{}-{repetition}", scenario.id),
        scenario.prompt.clone(),
    );
    input.env = Some(env);
    input.system_prompt = Some(scenario.system_instruction.clone());
    let mut trajectory = TrajectoryRecorder::new(started, ReportRedactor::from_environment());

    let terminal = match harness.submit(input).await {
        Ok(()) => {
            trajectory.record_submit_accepted();
            let outcome = tokio::time::timeout(agent.timeout(), async {
                while let Some(signal) = harness.next_signal().await {
                    if let Some(terminal) = trajectory.observe(signal) {
                        return terminal;
                    }
                }
                trajectory.record_signal_stream_ended();
                TrialTerminal::Cancelled
            })
            .await;
            if let Ok(outcome) = outcome {
                outcome
            } else {
                harness.cancel();
                trajectory.record_timeout(agent.timeout());
                TrialTerminal::TimedOut
            }
        }
        Err(error) => {
            let error = error.to_string();
            trajectory.record_submit_failed(&error);
            let invocations = read_invocations(&log_path).unwrap_or_default();
            drop(harness);
            drop(tmp);
            return TrialReport {
                scenario_id: scenario.id.clone(),
                prompt: scenario.prompt.clone(),
                system_instruction: scenario.system_instruction.clone(),
                agent,
                repetition,
                requested_model: None,
                resolved_model: trajectory.resolved_model,
                duration_ms: elapsed_ms(started),
                terminal_outcome: error,
                trajectory: trajectory.events,
                expectation_counts: expectation_counts(scenario, &invocations),
                invocations: redact_invocations(invocations),
                passed: false,
            };
        }
    };

    let invocations = read_invocations(&log_path).unwrap_or_default();
    let counts = expectation_counts(scenario, &invocations);
    let passed = terminal.is_completed() && expectations_pass(scenario, &counts);
    let terminal_outcome = terminal.label();
    drop(harness);
    drop(tmp);

    TrialReport {
        scenario_id: scenario.id.clone(),
        prompt: scenario.prompt.clone(),
        system_instruction: scenario.system_instruction.clone(),
        agent,
        repetition,
        requested_model: None,
        resolved_model: trajectory.resolved_model,
        duration_ms: elapsed_ms(started),
        terminal_outcome,
        trajectory: trajectory.events,
        invocations: redact_invocations(invocations),
        expectation_counts: counts,
        passed,
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn summarize(
    scenarios: &[Scenario],
    agents: &[EvalAgent],
    trials: &[TrialReport],
    required_passes: usize,
) -> Vec<AgentSummary> {
    scenarios
        .iter()
        .flat_map(|scenario| {
            agents.iter().map(move |agent| {
                let matching = trials
                    .iter()
                    .filter(|trial| trial.scenario_id == scenario.id && trial.agent == *agent)
                    .collect::<Vec<_>>();
                let passes = matching.iter().filter(|trial| trial.passed).count();
                AgentSummary {
                    scenario_id: scenario.id.clone(),
                    agent: *agent,
                    passes,
                    trials: matching.len(),
                    required_passes,
                    passed: passes >= required_passes,
                }
            })
        })
        .collect()
}

async fn cli_version(agent: EvalAgent) -> String {
    match Command::new(agent.cli_binary())
        .arg("--version")
        .output()
        .await
    {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            if stdout.is_empty() { stderr } else { stdout }
        }
        Err(error) => format!("unavailable: {error}"),
    }
}

async fn write_report(path: &Path, report: &EvalReport) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("report path has no parent: {}", path.display()))?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|error| format!("create report directory {}: {error}", parent.display()))?;
    let mut json = serde_json::to_string_pretty(report)
        .map_err(|error| format!("serialize evaluation report: {error}"))?;
    json.push('\n');
    tokio::fs::write(path, json)
        .await
        .map_err(|error| format!("write evaluation report {}: {error}", path.display()))
}
