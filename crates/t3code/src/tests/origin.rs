//! Which requests need an `Origin` header, and which exact value passes.

use super::*;

#[test]
fn origin_check_is_exact_and_requires_origin() {
    let allowed = vec![AllowedOrigin::parse("http://localhost:5733")];
    let mut headers = HeaderMap::new();
    assert!(!origin_allowed(&headers, &allowed));
    headers.insert(header::ORIGIN, "http://localhost:5733".parse().unwrap());
    assert!(origin_allowed(&headers, &allowed));
    headers.insert(header::ORIGIN, "http://attacker.example".parse().unwrap());
    assert!(!origin_allowed(&headers, &allowed));
}

#[test]
fn enabled_configuration_requires_an_origin() {
    let config = UiRpcConfig {
        environment_id: "moatless".into(),
        allowed_origins: vec![],
        otlp_traces_enabled: false,
        attachment_uploads: true,
        usage_limit_sources: false,
        orchestration_protocol_version: 1,
        review_guide_fork_author: false,
        scratch_workspace: None,
        linear_default_team: None,
    };
    assert!(config.validate().is_err());
}

#[test]
fn a_ticket_holder_needs_no_origin_and_everyone_else_does() {
    assert!(!origin_checked(Some(Extension(
        UiRpcAuthMethod::WebSocketTicket
    ))));
    assert!(origin_checked(Some(Extension(
        UiRpcAuthMethod::SessionCookie
    ))));
    assert!(origin_checked(Some(Extension(UiRpcAuthMethod::ApiKey))));
    // An upgrade the middleware never saw is treated as the cookie case.
    assert!(origin_checked(None));
}
