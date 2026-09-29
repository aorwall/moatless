//! Answering one pick decision: validation, admission, the provider call and
//! banding.

use std::collections::HashSet;
use std::time::Duration;

use moatless_api_schemas::decisions::{
    DecisionOption, OptionProbability, PickDecisionRequest, PickDecisionResponse,
};
use tokio::time::Instant;
use uuid::Uuid;

use crate::error::DecisionError;
use crate::provider::{Decider, DecisionProvider, ProviderPick, Question};

/// Covers queueing and every retry, and stays under the CLI's 30s HTTP timeout.
pub(crate) const DEADLINE: Duration = Duration::from_secs(20);
const QUEUE_WAIT: Duration = Duration::from_secs(5);
const MIN_OPTIONS: usize = 2;
const MAX_OPTIONS: usize = 255;
const MAX_OPTION_NAME_CHARS: usize = 128;

/// Who asked, for the decision's log line only.
pub(crate) struct Caller<'a> {
    pub(crate) user_id: Uuid,
    pub(crate) task_id: Option<&'a str>,
}

/// Picks one of `req.options` for `req.state`.
///
/// Rejects the request before calling the provider when it is invalid or
/// larger than the provider's input budget; nothing is truncated.
pub(crate) async fn pick(
    decider: &Decider,
    caller: Caller<'_>,
    req: &PickDecisionRequest,
) -> Result<PickDecisionResponse, DecisionError> {
    let started = Instant::now();
    let provider = &decider.provider;
    let result = answer(decider, req, started + DEADLINE).await;
    let elapsed = started.elapsed();
    let outcome = result
        .as_ref()
        .map_or_else(DecisionError::outcome, |_| "ok");
    crate::metrics::record_outcome(provider.name(), outcome, elapsed);
    let (response, attempts) = result?;
    crate::metrics::record_answer(provider.name(), response.band, response.usage);
    tracing::info!(
        provider = provider.name(),
        model = provider.model(),
        user_id = %caller.user_id,
        task_id = caller.task_id.unwrap_or("-"),
        options = req.options.len(),
        state_chars = req.state.chars().count(),
        choice = %response.choice,
        confidence = ?response.confidence,
        band = response.band.as_str(),
        input_tokens = response.usage.input_tokens,
        output_tokens = response.usage.output_tokens,
        attempts,
        elapsed_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
        "Decision picked"
    );
    Ok(response)
}

async fn answer(
    decider: &Decider,
    req: &PickDecisionRequest,
    deadline: Instant,
) -> Result<(PickDecisionResponse, u32), DecisionError> {
    let provider = &decider.provider;
    let options = validate(req, provider.max_input_chars())?;
    let question = Question {
        text: req.question.trim(),
        options: &options,
        state: &req.state,
    };
    let _permit = decider.admit(QUEUE_WAIT).await.ok_or(DecisionError::Busy)?;
    let pick = provider.pick(&question, deadline).await?;
    let attempts = pick.attempts;
    Ok((shape(provider, &options, pick)?, attempts))
}

/// Returns the options with trimmed names and descriptions.
pub(crate) fn validate(
    req: &PickDecisionRequest,
    max_input_chars: usize,
) -> Result<Vec<DecisionOption>, DecisionError> {
    let invalid = |message: String| Err(DecisionError::Invalid(message));
    let question = req.question.trim();
    if question.is_empty() {
        return invalid("question must not be empty".to_string());
    }
    if req.state.trim().is_empty() {
        return invalid("state must not be empty".to_string());
    }
    let count = req.options.len();
    if !(MIN_OPTIONS..=MAX_OPTIONS).contains(&count) {
        return invalid(format!(
            "between {MIN_OPTIONS} and {MAX_OPTIONS} options are required, got {count}"
        ));
    }
    let mut seen = HashSet::new();
    let mut options = Vec::with_capacity(count);
    for (index, option) in req.options.iter().enumerate() {
        let name = option.name.trim();
        let chars = name.chars().count();
        if chars == 0 || chars > MAX_OPTION_NAME_CHARS {
            return invalid(format!(
                "option {} has a {chars}-character name; names must be 1 to {MAX_OPTION_NAME_CHARS} characters",
                index + 1
            ));
        }
        if name.chars().any(char::is_control) {
            return invalid(format!("option {name:?} contains a control character"));
        }
        if !seen.insert(name) {
            return invalid(format!("option {name:?} is listed twice"));
        }
        options.push(DecisionOption {
            name: name.to_string(),
            description: option
                .description
                .as_deref()
                .map(str::trim)
                .filter(|d| !d.is_empty())
                .map(str::to_string),
        });
    }
    let size = question.chars().count()
        + req.state.chars().count()
        + options
            .iter()
            .map(|o| {
                o.name.chars().count() + o.description.as_ref().map_or(0, |d| d.chars().count())
            })
            .sum::<usize>();
    if size > max_input_chars {
        return invalid(format!(
            "question, state and options total {size} characters; the limit is {max_input_chars}, so shorten the state"
        ));
    }
    Ok(options)
}

/// Checks the choice was offered, bands it and orders the probabilities most
/// likely first, ties in option order.
pub(crate) fn shape(
    provider: &DecisionProvider,
    options: &[DecisionOption],
    pick: ProviderPick,
) -> Result<PickDecisionResponse, DecisionError> {
    if !options.iter().any(|o| o.name == pick.choice) {
        return Err(DecisionError::UnknownChoice);
    }
    let mut probabilities: Vec<OptionProbability> = options
        .iter()
        .filter_map(|o| {
            let (_, probability) = pick
                .probabilities
                .iter()
                .find(|(name, _)| *name == o.name)?;
            Some(OptionProbability {
                name: o.name.clone(),
                probability: *probability,
            })
        })
        .collect();
    probabilities.sort_by(|a, b| b.probability.total_cmp(&a.probability));
    Ok(PickDecisionResponse {
        band: provider.band(pick.confidence),
        choice: pick.choice,
        confidence: pick.confidence,
        probabilities,
        provider: provider.name().to_string(),
        model: provider.model().to_string(),
        usage: pick.usage,
    })
}
