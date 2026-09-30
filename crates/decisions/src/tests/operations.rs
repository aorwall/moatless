use std::time::Duration;

use moatless_api_schemas::decisions::{
    DecisionBand, DecisionOption, DecisionUsage, PickDecisionRequest,
};

use super::stub::{UNREACHABLE, jev_config};
use crate::error::DecisionError;
use crate::jev::{JevClient, MAX_INPUT_CHARS};
use crate::operations::{shape, validate};
use crate::provider::{Decider, DecisionProvider, ProviderPick};

fn request(names: &[&str], state: &str) -> PickDecisionRequest {
    PickDecisionRequest {
        question: "Which one?".to_string(),
        options: names
            .iter()
            .map(|name| DecisionOption {
                name: (*name).to_string(),
                description: None,
            })
            .collect(),
        state: state.to_string(),
        task_id: None,
    }
}

fn jev() -> DecisionProvider {
    DecisionProvider::Jev(JevClient::new(
        &jev_config(UNREACHABLE),
        moatless_core::config::HttpConfig::default().decision,
    ))
}

fn invalid_message(req: &PickDecisionRequest) -> String {
    match validate(req, MAX_INPUT_CHARS) {
        Err(DecisionError::Invalid(message)) => message,
        other => panic!("expected Invalid, got {other:?}"),
    }
}

#[test]
fn validation_rejects_each_malformed_request_with_a_reason() {
    let long_name = "x".repeat(129);
    let many: Vec<String> = (0..256).map(|i| format!("o{i}")).collect();
    let many: Vec<&str> = many.iter().map(String::as_str).collect();
    let mut blank_question = request(&["a", "b"], "text");
    blank_question.question = "  ".to_string();
    let cases = [
        (request(&["a"], "text"), "between 2 and 255"),
        (request(&many, "text"), "got 256"),
        (blank_question, "question must not be empty"),
        (request(&["a", "b"], " \n "), "state must not be empty"),
        (request(&["a", " a "], "text"), "listed twice"),
        (
            request(&["a", "  "], "text"),
            "option 2 has a 0-character name",
        ),
        (request(&["a", &long_name], "text"), "129-character name"),
        (request(&["a", "b\tc"], "text"), "control character"),
    ];
    for (req, expected) in cases {
        let message = invalid_message(&req);
        assert!(message.contains(expected), "{message:?} lacks {expected:?}");
    }
}

#[test]
fn validation_states_the_size_and_the_limit_when_over_budget() {
    let message = invalid_message(&request(&["a", "b"], &"s".repeat(MAX_INPUT_CHARS)));

    assert!(
        message.contains(&format!("total {}", MAX_INPUT_CHARS + 12)),
        "{message}"
    );
    assert!(
        message.contains(&format!("limit is {MAX_INPUT_CHARS}")),
        "{message}"
    );
}

#[test]
fn validation_trims_names_and_drops_blank_descriptions() {
    let mut req = request(&[" a ", "b"], "text");
    req.options[0].description = Some("  first  ".to_string());
    req.options[1].description = Some("   ".to_string());

    let options = validate(&req, MAX_INPUT_CHARS).expect("valid");

    assert_eq!(
        options,
        vec![
            DecisionOption {
                name: "a".to_string(),
                description: Some("first".to_string())
            },
            DecisionOption {
                name: "b".to_string(),
                description: None
            },
        ]
    );
}

#[test]
fn jev_bands_follow_its_thresholds_and_a_missing_confidence_escalates() {
    let provider = jev();
    for (confidence, band) in [
        (Some(1.0), DecisionBand::Act),
        (Some(0.75), DecisionBand::Act),
        (Some(0.7499), DecisionBand::Confirm),
        (Some(0.45), DecisionBand::Confirm),
        (Some(0.4499), DecisionBand::Escalate),
        (None, DecisionBand::Escalate),
    ] {
        assert_eq!(provider.band(confidence), band, "{confidence:?}");
    }
}

fn provider_pick(choice: &str, probabilities: &[(&str, f64)]) -> ProviderPick {
    ProviderPick {
        choice: choice.to_string(),
        confidence: Some(0.5),
        probabilities: probabilities
            .iter()
            .map(|(name, p)| ((*name).to_string(), *p))
            .collect(),
        usage: DecisionUsage::default(),
        attempts: 1,
    }
}

#[test]
fn probabilities_sort_most_likely_first_ties_in_option_order_unknown_names_dropped() {
    let options =
        validate(&request(&["a", "b", "c", "d"], "text"), MAX_INPUT_CHARS).expect("valid");
    let pick = provider_pick(
        "c",
        &[("d", 0.2), ("c", 0.5), ("zzz", 0.9), ("b", 0.2), ("a", 0.1)],
    );

    let response = shape(&jev(), &options, pick).expect("shaped");

    let order: Vec<(&str, f64)> = response
        .probabilities
        .iter()
        .map(|p| (p.name.as_str(), p.probability))
        .collect();
    assert_eq!(order, vec![("c", 0.5), ("b", 0.2), ("d", 0.2), ("a", 0.1)]);
    assert_eq!(response.band, DecisionBand::Confirm);
    assert_eq!(response.provider, "jev");
    assert_eq!(response.model, "jev-1.13.0");
}

#[test]
fn a_choice_that_was_not_offered_is_an_error() {
    let options = validate(&request(&["a", "b"], "text"), MAX_INPUT_CHARS).expect("valid");

    let error = shape(&jev(), &options, provider_pick("c", &[])).expect_err("unknown");

    assert!(matches!(error, DecisionError::UnknownChoice), "{error:?}");
}

#[tokio::test]
async fn admission_fails_when_every_slot_stays_taken() {
    let decider = Decider::new(jev(), 1);
    let held = decider.admit(Duration::from_millis(10)).await;
    assert!(held.is_some());

    assert!(decider.admit(Duration::from_millis(10)).await.is_none());
    drop(held);
    assert!(decider.admit(Duration::from_millis(10)).await.is_some());
}
