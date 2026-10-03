//! Failure classification: startup, stream and turn errors.

use chrono::TimeZone;

use super::super::super::classify::try_again_at;
use super::fixtures::*;
use super::*;

#[test]
fn startup_spawn_failure_is_invalid_configuration() {
    let error = std::io::Error::new(std::io::ErrorKind::NotFound, "codex: not found");
    let classified = classify_startup_error(&error);
    assert_eq!(classified.class, ErrorClass::InvalidConfiguration);
    assert_eq!(classified.stage, FailureStage::Startup);
}

#[test]
fn stream_parse_error_is_native_protocol() {
    let classified = classify_stream_error("Failed to parse an app-server frame: bad");
    assert_eq!(classified.class, ErrorClass::NativeProtocol);
    assert_eq!(classified.stage, FailureStage::Execution);
}

#[test]
fn stream_exit_error_is_process_exit() {
    let classified = classify_stream_error("Codex app-server exited with status 1");
    assert_eq!(classified.class, ErrorClass::ProcessExit);
}

/// The whole `codexErrorInfo` table, in the wire's own spelling.
#[test]
fn every_named_turn_error_maps_to_its_class_and_impact() {
    let cases: &[(Value, ErrorClass, SessionImpact)] = &[
        (
            json!("unauthorized"),
            ErrorClass::Authentication,
            SessionImpact::Unknown,
        ),
        (
            json!("usageLimitExceeded"),
            ErrorClass::UsageLimit,
            SessionImpact::Intact,
        ),
        (
            json!("serverOverloaded"),
            ErrorClass::RateLimit,
            SessionImpact::Intact,
        ),
        (
            json!("internalServerError"),
            ErrorClass::RateLimit,
            SessionImpact::Intact,
        ),
        (
            json!({ "httpConnectionFailed": { "httpStatusCode": 502 } }),
            ErrorClass::RateLimit,
            SessionImpact::Intact,
        ),
        (
            json!({ "responseStreamConnectionFailed": { "httpStatusCode": 503 } }),
            ErrorClass::RateLimit,
            SessionImpact::Intact,
        ),
        (
            json!({ "responseStreamDisconnected": { "httpStatusCode": null } }),
            ErrorClass::Connectivity,
            SessionImpact::Intact,
        ),
        (
            json!({ "responseTooManyFailedAttempts": { "httpStatusCode": 500 } }),
            ErrorClass::RateLimit,
            SessionImpact::Intact,
        ),
        (
            json!({ "responseTooManyFailedAttempts": { "httpStatusCode": 429 } }),
            ErrorClass::ProviderThrottle,
            SessionImpact::Intact,
        ),
        (
            json!("sandboxError"),
            ErrorClass::InvalidConfiguration,
            SessionImpact::Intact,
        ),
        (
            json!("threadRollbackFailed"),
            ErrorClass::UnavailableSession,
            SessionImpact::Lost,
        ),
        (
            json!("contextWindowExceeded"),
            ErrorClass::InvalidInput,
            SessionImpact::Intact,
        ),
        (
            json!("sessionBudgetExceeded"),
            ErrorClass::InvalidInput,
            SessionImpact::Intact,
        ),
        (
            json!("badRequest"),
            ErrorClass::InvalidInput,
            SessionImpact::Intact,
        ),
        (
            json!("cyberPolicy"),
            ErrorClass::InvalidInput,
            SessionImpact::Intact,
        ),
        (
            json!("other"),
            ErrorClass::InvalidInput,
            SessionImpact::Intact,
        ),
        (
            json!({ "activeTurnNotSteerable": { "turnKind": "review" } }),
            ErrorClass::InvalidInput,
            SessionImpact::Intact,
        ),
        // Not in this build's table: conservative default, and logged with its
        // raw JSON so a new upstream code is visible.
        (
            json!("someBrandNewCode"),
            ErrorClass::InvalidInput,
            SessionImpact::Intact,
        ),
        (
            json!({ "brandNewObjectCode": { "detail": 1 } }),
            ErrorClass::InvalidInput,
            SessionImpact::Intact,
        ),
    ];

    for (info, class, impact) in cases {
        let error = turn_error(json!({ "message": "boom", "codexErrorInfo": info }));
        let classified = classify_turn_error(&error);
        assert_eq!(classified.class, *class, "codexErrorInfo {info}");
        assert_eq!(classified.session_impact, *impact, "codexErrorInfo {info}");
    }
}

/// The same three causes carry an optional status code — the only signal for
/// whether a server answered.
///
/// With none, nothing reached the provider and the pod's egress is the suspect,
/// so the class says connectivity rather than blaming the provider's capacity.
#[test]
fn a_connection_failure_without_a_status_code_is_connectivity() {
    for name in [
        "httpConnectionFailed",
        "responseStreamConnectionFailed",
        "responseStreamDisconnected",
    ] {
        let info = json!({ name: { "httpStatusCode": null } });
        let classified = classify_turn_error(&turn_error(
            json!({ "message": "boom", "codexErrorInfo": info }),
        ));
        assert_eq!(classified.class, ErrorClass::Connectivity, "{name}");
        assert_eq!(classified.session_impact, SessionImpact::Intact, "{name}");
    }
}

#[test]
fn a_turn_error_with_no_named_cause_takes_the_conservative_default() {
    let classified = classify_turn_error(&turn_error(json!({ "message": "boom" })));
    assert_eq!(classified.class, ErrorClass::InvalidInput);
    assert_eq!(classified.session_impact, SessionImpact::Intact);
}

#[test]
fn a_turn_errors_additional_details_reach_the_diagnostic() {
    let classified = classify_turn_error(&turn_error(json!({
        "message": "request failed",
        "additionalDetails": "upstream returned 429",
    })));
    assert!(
        classified.diagnostic.contains("upstream returned 429"),
        "{}",
        classified.diagnostic
    );
}

#[test]
fn only_an_unauthorized_turn_means_the_credential_expired() {
    assert!(is_expired_credential(&turn_error(
        json!({ "message": "no", "codexErrorInfo": "unauthorized" })
    )));
    assert!(!is_expired_credential(&turn_error(
        json!({ "message": "no", "codexErrorInfo": "badRequest" })
    )));
    assert!(!is_expired_credential(&turn_error(
        json!({ "message": "no" })
    )));
}

/// The coordinator polls this flag at the turn-failure boundary and owns the
/// pull/compare/retry from there; the harness only reports the fact.
#[test]
fn an_unauthorized_turn_flags_the_codex_credential() {
    let cfg = config();
    let detector = cfg.credential_failures.clone();
    let mut p = CodexProcess::new(cfg);
    p.absorb(thread_started("thread-A"));
    drain(&mut p);

    p.absorb(turn_completed(
        "failed",
        Some(json!({ "message": "unauthorized", "codexErrorInfo": "unauthorized" })),
    ));
    let events = drain(&mut p);
    match events.last() {
        Some(ProcessEvent::Terminated {
            termination: ProcessTermination::Failed(error),
        }) => assert_eq!(error.class, ErrorClass::Authentication),
        other => panic!("expected an authentication failure, got {other:?}"),
    }
    assert!(detector.take(CODEX_AUTH_JSON), "credential must be flagged");
}

#[test]
fn a_turn_that_fails_for_another_reason_leaves_the_credential_alone() {
    let cfg = config();
    let detector = cfg.credential_failures.clone();
    let mut p = CodexProcess::new(cfg);
    p.absorb(turn_completed(
        "failed",
        Some(json!({ "message": "nope", "codexErrorInfo": "badRequest" })),
    ));
    drain(&mut p);
    assert!(!detector.take(CODEX_AUTH_JSON));
}

/// The provider's own sentence, as Codex renders `UsageLimitReachedError`, with
/// the reset time it states in the pod's local zone.
#[test]
fn a_usage_limit_carries_the_reset_time_the_provider_states() {
    let error = turn_error(json!({
        "message": "You\u{2019}ve hit your usage limit. Upgrade to Pro \
                    (https://chatgpt.com/explore/pro) or try again at Oct 3rd, 2026 6:05 PM.",
        "codexErrorInfo": "usageLimitExceeded"
    }));
    let classified = classify_turn_error(&error);
    assert_eq!(classified.class, ErrorClass::UsageLimit);
    assert!(!crate::agent::retry::is_retryable_harness_error(
        &classified
    ));
    let expected = chrono::Local
        .with_ymd_and_hms(2026, 10, 3, 18, 5, 0)
        .single()
        .expect("unambiguous local time")
        .with_timezone(&chrono::Utc);
    assert_eq!(classified.resets_at, Some(expected));
    assert!(
        classified
            .diagnostic
            .contains(&expected.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
        "{}",
        classified.diagnostic
    );
}

#[test]
fn a_usage_limit_message_without_a_named_cause_is_a_usage_limit() {
    for info in [json!(null), json!("other"), json!("someBrandNewCode")] {
        let classified = classify_turn_error(&turn_error(json!({
            "message": "You've hit your usage limit. Try again later.",
            "codexErrorInfo": info
        })));
        assert_eq!(classified.class, ErrorClass::UsageLimit, "{info}");
        assert_eq!(classified.resets_at, None, "{info}");
    }
}

#[test]
fn a_same_day_reset_reads_against_today() {
    let now = chrono::Utc.with_ymd_and_hms(2026, 10, 1, 20, 0, 0).unwrap();
    assert_eq!(
        try_again_at("You've hit your usage limit. Try again at 11:30 PM.", now),
        Some(
            chrono::Utc
                .with_ymd_and_hms(2026, 10, 1, 23, 30, 0)
                .unwrap()
        )
    );
    assert_eq!(
        try_again_at("or try again at Oct 3rd, 2026 6:05 PM.", now),
        Some(chrono::Utc.with_ymd_and_hms(2026, 10, 3, 18, 5, 0).unwrap())
    );
    assert_eq!(try_again_at("Try again later.", now), None);
}

/// A `CLIProxyAPI` hub's 503 when no pooled account can serve the model,
/// however Codex names the failure around it.
#[test]
fn a_hub_with_no_account_is_provider_unavailable() {
    let message = "unexpected status 503 Service Unavailable: auth_unavailable: no auth available (providers=codex)";
    for info in [
        Value::Null,
        json!("other"),
        json!({ "responseTooManyFailedAttempts": { "httpStatusCode": 503 } }),
    ] {
        let classified = classify_turn_error(&turn_error(
            json!({ "message": message, "codexErrorInfo": info }),
        ));
        assert_eq!(classified.class, ErrorClass::ProviderUnavailable, "{info}");
    }
}

/// A safety refusal never reads as a limit or an outage, so it is neither
/// retried nor routed to the other provider.
#[test]
fn a_safety_refusal_is_invalid_input_whatever_else_it_names() {
    let message = "Our safeguards flagged this message (usage limit policy); unexpected status 503";
    for info in [
        Value::Null,
        json!("other"),
        json!({ "responseTooManyFailedAttempts": { "httpStatusCode": 503 } }),
    ] {
        let classified = classify_turn_error(&turn_error(
            json!({ "message": message, "codexErrorInfo": info }),
        ));
        assert_eq!(classified.class, ErrorClass::InvalidInput, "{info}");
    }
}
