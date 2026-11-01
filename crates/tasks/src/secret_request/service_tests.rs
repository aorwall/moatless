use super::*;

fn ctx(user_id: &str, is_admin: bool) -> ScopeContext {
    ScopeContext::new(user_id.to_string(), is_admin)
}

fn new_request(key: &str, scope: Scope, kind: SecretKind) -> NewSecretRequest {
    NewSecretRequest {
        key: key.to_string(),
        scope,
        kind,
        label: None,
        reason: "To call the staging API".to_string(),
    }
}

#[test]
fn a_request_is_made_by_its_caller_and_labelled_with_its_key_by_default() {
    let asked = asked(
        new_request("STAGING_API_KEY", Scope::User, SecretKind::Env),
        &ctx("u1", false),
    )
    .unwrap();
    assert_eq!(asked.key, "STAGING_API_KEY");
    assert_eq!(asked.label, "STAGING_API_KEY");
    assert_eq!(asked.requested_by, "u1");
    assert!(Uuid::parse_str(&asked.request_id).is_ok());
}

#[test]
fn platform_credentials_cannot_be_requested() {
    for key in [
        "GITHUB_TOKEN",
        "GH_TOKEN",
        "ANTHROPIC_API_KEY",
        "CLAUDE_CODE_OAUTH_TOKEN",
        "OPENAI_API_KEY",
        "FORGEJO_TOKEN_GIT_EXAMPLE_COM",
        "USER_AUTH_TOKEN",
        "AWS_SECRET_ACCESS_KEY",
    ] {
        let refused = asked(
            new_request(key, Scope::User, SecretKind::Env),
            &ctx("u1", false),
        );
        assert!(refused.is_err(), "{key} should be refused");
    }
}

#[test]
fn a_request_asks_only_for_env_or_runtime_file_secrets() {
    assert!(
        asked(
            new_request("KEY", Scope::User, SecretKind::RuntimeFile),
            &ctx("u1", false)
        )
        .is_ok()
    );
    for kind in [
        SecretKind::ProviderToken,
        SecretKind::Deployment,
        SecretKind::Internal,
    ] {
        assert!(asked(new_request("KEY", Scope::User, kind), &ctx("u1", false)).is_err());
    }
}

#[test]
fn an_invalid_key_or_an_oversized_label_is_refused() {
    assert!(
        asked(
            new_request("lower_case", Scope::User, SecretKind::Env),
            &ctx("u1", false)
        )
        .is_err()
    );
    let mut long = new_request("KEY", Scope::User, SecretKind::Env);
    long.label = Some("x".repeat(MAX_LABEL_CHARS + 1));
    assert!(asked(long, &ctx("u1", false)).is_err());
}

#[test]
fn only_the_requester_answers_a_user_request() {
    let asked = asked(
        new_request("KEY", Scope::User, SecretKind::Env),
        &ctx("u1", false),
    )
    .unwrap();
    assert!(may_answer(&ctx("u1", false), &asked).is_ok());
    assert!(may_answer(&ctx("u2", false), &asked).is_err());
    // An admin saves to their own user scope, which this Sandbox never reads.
    assert!(may_answer(&ctx("u2", true), &asked).is_err());
}

#[test]
fn only_an_admin_answers_a_global_request() {
    let asked = asked(
        new_request("KEY", Scope::Global, SecretKind::Env),
        &ctx("u1", false),
    )
    .unwrap();
    assert!(may_answer(&ctx("u1", false), &asked).is_err());
    assert!(may_answer(&ctx("u2", true), &asked).is_ok());
}

#[test]
fn neither_the_notice_nor_the_answers_debug_carries_the_value() {
    let asked = asked(
        new_request("KEY", Scope::User, SecretKind::Env),
        &ctx("u1", false),
    )
    .unwrap();
    let answered = SecretRequestAnswered {
        request_id: asked.request_id.clone(),
        outcome: SecretRequestOutcome::Saved,
        answered_by: "u1".to_string(),
        answered_by_login: "alice".to_string(),
    };
    let text = notice_text(&asked, &answered);
    assert!(text.contains("alice saved the secret `KEY` to the user secrets"));
    assert_eq!(
        format!("{:?}", Answer::Save("hunter2".to_string())),
        "Save([REDACTED])"
    );
}
