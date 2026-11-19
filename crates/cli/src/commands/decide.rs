//! `moat decide` — cheap multiple-choice decisions from the backend's decision
//! provider.

use std::path::PathBuf;

use clap::{ArgGroup, Args, Subcommand};
use futures_util::{Stream, StreamExt};
use moatless_api_client::{ApiClient, ApiError};
use moatless_api_schemas::decisions::{DecisionOption, PickDecisionRequest, PickDecisionResponse};
use serde_json::Value;

use crate::client::MoatClient;
use crate::commands::input::read_text_or_stdin;
use crate::config::Config;
use crate::output;

/// Decisions in flight at once for `--each`.
const EACH_CONCURRENCY: usize = 4;
const TSV_HEADER: &str = "id\tchoice\tconfidence\tband\trunner_up";

#[derive(Subcommand)]
pub enum DecideAction {
    /// Pick one option for a piece of text: route, label or triage it.
    ///
    /// The question, options, descriptions and text all go to an external model
    /// provider: never pass secrets, credentials or personal data. Act on band `act`, check `confirm`, and
    /// decide `escalate` yourself.
    Pick(Box<PickArgs>),
}

#[derive(Args)]
#[command(group(ArgGroup::new("choices").required(true).multiple(true).args(["options", "option"])))]
pub struct PickArgs {
    /// What to decide, e.g. "Which repository owns this issue?"
    pub question: String,
    /// Comma-separated option names
    #[arg(long, value_delimiter = ',', value_name = "A,B,...")]
    pub options: Vec<String>,
    /// One option, with an optional description of when it applies; repeatable
    #[arg(long, value_name = "NAME[=DESCRIPTION]")]
    pub option: Vec<String>,
    /// The text to decide about (with `--each`, the batch). Defaults to stdin
    #[arg(long, conflicts_with = "state_file")]
    pub state: Option<String>,
    /// Read the text to decide about (with `--each`, the batch) from a file
    #[arg(long, value_name = "PATH")]
    pub state_file: Option<PathBuf>,
    /// Decide once per item. The input is JSONL or a JSON array whose items
    /// are strings or `{"id": ..., "state": "..."}` objects; prints one TSV
    /// row per item, in input order
    #[arg(long)]
    pub each: bool,
}

/// One `--each` input item; `id` defaults to its 1-based position.
#[derive(Debug, PartialEq, Eq)]
pub struct BatchItem {
    pub id: String,
    pub state: String,
}

pub async fn run(
    moat: &MoatClient,
    action: DecideAction,
    cfg: &Config,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let DecideAction::Pick(args) = action;
    let PickArgs {
        question,
        options,
        option,
        state,
        state_file,
        each,
    } = *args;
    let input = match state_file {
        Some(path) => std::fs::read_to_string(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?,
        None => read_text_or_stdin(state, "Pass the text with --state or --state-file")?,
    };
    let template = PickDecisionRequest {
        question,
        options: parse_options(&options, &option),
        state: String::new(),
        task_id: cfg.task_id().map(str::to_string),
    };
    if !each {
        let response = moat
            .api()
            .pick_decision(&PickDecisionRequest {
                state: input,
                ..template
            })
            .await?;
        output::emit(json, &response, || println!("{}", summary(&response)));
        return Ok(());
    }

    let items = parse_batch(&input)?;
    let total = items.len();
    let mut failed = 0;
    let mut unauthorized = false;
    if !json {
        println!("{TSV_HEADER}");
    }
    let mut results = std::pin::pin!(pick_each(moat.api(), &template, items));
    while let Some((id, result)) = results.next().await {
        if let Err(e) = &result {
            failed += 1;
            unauthorized |= matches!(e, ApiError::Unauthorized);
        }
        if json {
            println!("{}", jsonl_row(&id, &result));
        } else {
            println!("{}", tsv_row(&id, &result));
        }
    }
    // Surfaced as the typed error so the caller drops a stale session.
    if unauthorized {
        return Err(ApiError::Unauthorized.into());
    }
    if failed > 0 {
        return Err(format!("{failed} of {total} decisions failed").into());
    }
    Ok(())
}

/// `--options` names first, then each `--option`, split on its first `=`.
pub fn parse_options(names: &[String], options: &[String]) -> Vec<DecisionOption> {
    let listed = names.iter().map(|name| DecisionOption {
        name: name.clone(),
        description: None,
    });
    let described = options.iter().map(|raw| match raw.split_once('=') {
        Some((name, description)) => DecisionOption {
            name: name.to_string(),
            description: Some(description.to_string()),
        },
        None => DecisionOption {
            name: raw.clone(),
            description: None,
        },
    });
    listed.chain(described).collect()
}

/// Parses `--each` input: a JSON array when it starts with `[`, else JSONL
/// with blank lines skipped.
pub fn parse_batch(input: &str) -> Result<Vec<BatchItem>, String> {
    let values: Vec<(String, Value)> = if input.trim_start().starts_with('[') {
        let array: Vec<Value> =
            serde_json::from_str(input).map_err(|e| format!("--each input: {e}"))?;
        array
            .into_iter()
            .enumerate()
            .map(|(i, value)| (format!("item {}", i + 1), value))
            .collect()
    } else {
        input
            .lines()
            .enumerate()
            .filter(|(_, line)| !line.trim().is_empty())
            .map(|(i, line)| {
                serde_json::from_str(line)
                    .map(|value| (format!("line {}", i + 1), value))
                    .map_err(|e| format!("--each input line {}: {e}", i + 1))
            })
            .collect::<Result<_, _>>()?
    };
    if values.is_empty() {
        return Err("--each input has no items".to_string());
    }
    values
        .into_iter()
        .enumerate()
        .map(|(position, (at, value))| {
            batch_item(position + 1, value).ok_or_else(|| {
                format!("--each input {at}: expected a string or an object with a string \"state\"")
            })
        })
        .collect()
}

fn batch_item(position: usize, value: Value) -> Option<BatchItem> {
    match value {
        Value::String(state) => Some(BatchItem {
            id: position.to_string(),
            state,
        }),
        Value::Object(mut object) => {
            let Value::String(state) = object.remove("state")? else {
                return None;
            };
            let id = match object.remove("id") {
                None | Some(Value::Null) => position.to_string(),
                Some(Value::String(id)) => id,
                Some(other) => other.to_string(),
            };
            Some(BatchItem { id, state })
        }
        _ => None,
    }
}

/// Decides every item with the same question and options; yields in input
/// order with [`EACH_CONCURRENCY`] requests in flight.
pub fn pick_each<'a>(
    api: &'a ApiClient,
    template: &'a PickDecisionRequest,
    items: Vec<BatchItem>,
) -> impl Stream<Item = (String, Result<PickDecisionResponse, ApiError>)> + 'a {
    futures_util::stream::iter(items)
        .map(move |item| async move {
            let request = PickDecisionRequest {
                state: item.state,
                ..template.clone()
            };
            (item.id, api.pick_decision(&request).await)
        })
        .buffered(EACH_CONCURRENCY)
}

pub fn summary(response: &PickDecisionResponse) -> String {
    let confidence = response.confidence.map_or_default(|c| format!("{c:.2}, "));
    let mut lines = vec![format!(
        "choice: {} ({confidence}{})",
        response.choice,
        response.band.as_str()
    )];
    let alternatives: Vec<String> = response
        .probabilities
        .iter()
        .filter(|p| p.name != response.choice)
        .take(3)
        .map(|p| format!("{} {:.2}", p.name, p.probability))
        .collect();
    if !alternatives.is_empty() {
        lines.push(format!("alternatives: {}", alternatives.join(", ")));
    }
    lines.push(format!(
        "usage: {}/{} · {} input tokens",
        response.provider, response.model, response.usage.input_tokens
    ));
    lines.join("\n")
}

/// An error row has band `error` and the message in the last column.
pub fn tsv_row(id: &str, result: &Result<PickDecisionResponse, ApiError>) -> String {
    let columns = match result {
        Ok(response) => [
            id.to_string(),
            response.choice.clone(),
            response
                .confidence
                .map_or_else(|| "-".to_string(), |c| format!("{c:.2}")),
            response.band.as_str().to_string(),
            response
                .probabilities
                .iter()
                .find(|p| p.name != response.choice)
                .map_or_else(|| "-".to_string(), |p| p.name.clone()),
        ],
        Err(e) => [
            id.to_string(),
            "-".to_string(),
            "-".to_string(),
            "error".to_string(),
            error_message(e),
        ],
    };
    columns.map(|c| tsv_cell(&c)).join("\t")
}

pub fn jsonl_row(id: &str, result: &Result<PickDecisionResponse, ApiError>) -> Value {
    match result {
        Ok(response) => serde_json::json!({"id": id, "result": response}),
        Err(e) => serde_json::json!({"id": id, "error": error_message(e)}),
    }
}

/// The backend's `{"error": ...}` message when there is one.
fn error_message(error: &ApiError) -> String {
    if let ApiError::Api { status, body } = error {
        if let Some(message) = serde_json::from_str::<Value>(body)
            .ok()
            .and_then(|v| v["error"].as_str().map(str::to_string))
        {
            return format!("{status}: {message}");
        }
    }
    error.to_string()
}

fn tsv_cell(value: &str) -> String {
    value
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

#[cfg(test)]
mod tests;
