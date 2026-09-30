//! What the environment parses into: CSRF origins, `AuthProvider`, `env_bool`,
//! validation, and the `Debug` redaction that keeps secrets out of logs.

use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::{Mutex, PoisonError};

use super::*;

// =========================================================================
// CSRF trusted origins
// =========================================================================

/// Restore whatever the process had, so this test does not leak into the others
/// sharing the environment.
const ORIGIN_VARS: [&str; 2] = ["CSRF_TRUSTED_ORIGINS", "T3_UI_RPC_ALLOWED_ORIGINS"];

/// The environment is process-wide and `cargo test` runs a binary's tests on
/// several threads, so the two tests below would otherwise interleave: one sets
/// `CSRF_TRUSTED_ORIGINS` while the other asserts it is absent.
static ORIGIN_ENV_LOCK: Mutex<()> = Mutex::new(());

fn apply_origin_vars(values: [Option<String>; 2]) {
    for (name, value) in ORIGIN_VARS.into_iter().zip(values) {
        match value {
            Some(value) => std::env::set_var(name, value),
            None => std::env::remove_var(name),
        }
    }
}

/// Run `check` with the two variables set, restoring what the process had.
///
/// Restores through a panic too — a leaked variable would fail an unrelated
/// test later, far from the cause.
fn with_origin_vars(csrf: Option<&str>, websocket: Option<&str>, check: impl FnOnce()) {
    // A failing test poisons the lock. The guarded data is `()`, so there is
    // no torn state to protect the next test from.
    let _guard = ORIGIN_ENV_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner);

    let previous = ORIGIN_VARS.map(|name| std::env::var(name).ok());
    apply_origin_vars([csrf.map(str::to_string), websocket.map(str::to_string)]);

    let outcome = catch_unwind(AssertUnwindSafe(check));

    apply_origin_vars(previous);
    if let Err(panic) = outcome {
        resume_unwind(panic);
    }
}

/// The WebSocket allow-list must not reach the CSRF check: it carries wildcard
/// preview origins, and a preview serves agent-authored code.
#[test]
fn the_websocket_allow_list_is_not_a_source_of_csrf_trust() {
    with_origin_vars(
        None,
        Some("https://*.preview.example,https://t3.example"),
        || assert!(csrf_trusted_origins_from_env().is_empty()),
    );
}

#[test]
fn csrf_trust_is_read_from_its_own_variable() {
    with_origin_vars(Some("https://t3.example"), None, || {
        assert_eq!(
            csrf_trusted_origins_from_env(),
            vec![AllowedOrigin::Exact("https://t3.example".into())]
        );
    });
}

// =========================================================================
// AuthProvider parsing
// =========================================================================

#[test]
fn auth_provider_from_str_github_returns_error() {
    assert!("github".parse::<AuthProvider>().is_err());
}

#[test]
fn auth_provider_from_str_auth0() {
    assert_eq!(
        "auth0".parse::<AuthProvider>().unwrap(),
        AuthProvider::Auth0
    );
    assert_eq!(
        "Auth0".parse::<AuthProvider>().unwrap(),
        AuthProvider::Auth0
    );
}

#[test]
fn auth_provider_from_str_unknown_returns_error() {
    assert!("unknown".parse::<AuthProvider>().is_err());
    // A typo must fail closed rather than silently disabling auth.
    assert!("guthub".parse::<AuthProvider>().is_err());
}

#[test]
fn auth_provider_display() {
    assert_eq!(AuthProvider::GitHubApp.to_string(), "github_app");
    assert_eq!(AuthProvider::Auth0.to_string(), "auth0");
    assert_eq!(AuthProvider::Oidc.to_string(), "oidc");
}

#[test]
fn config_defaults_codex_model_to_gpt_56_sol() {
    let previous = std::env::var("DEFAULT_CODEX_MODEL").ok();
    std::env::remove_var("DEFAULT_CODEX_MODEL");
    let model = env_str("DEFAULT_CODEX_MODEL", DEFAULT_CODEX_MODEL);
    match previous {
        Some(value) => std::env::set_var("DEFAULT_CODEX_MODEL", value),
        None => std::env::remove_var("DEFAULT_CODEX_MODEL"),
    }
    assert_eq!(model, DEFAULT_CODEX_MODEL);
}

// =========================================================================
// env_bool logic
// =========================================================================

#[test]
fn env_bool_default_false_recognises_true_values() {
    // With default=false, only "true" and "1" return true
    std::env::set_var("_TEST_BOOL_1", "true");
    assert!(env_bool("_TEST_BOOL_1", false));

    std::env::set_var("_TEST_BOOL_2", "1");
    assert!(env_bool("_TEST_BOOL_2", false));

    std::env::set_var("_TEST_BOOL_3", "false");
    assert!(!env_bool("_TEST_BOOL_3", false));

    std::env::set_var("_TEST_BOOL_4", "anything");
    assert!(!env_bool("_TEST_BOOL_4", false));

    // Missing env var
    std::env::remove_var("_TEST_BOOL_MISSING");
    assert!(!env_bool("_TEST_BOOL_MISSING", false));

    // Cleanup
    for k in [
        "_TEST_BOOL_1",
        "_TEST_BOOL_2",
        "_TEST_BOOL_3",
        "_TEST_BOOL_4",
    ] {
        std::env::remove_var(k);
    }
}

#[test]
fn env_bool_default_true_recognises_false_values() {
    // With default=true, only "false" and "0" return false
    std::env::set_var("_TEST_BOOL_T1", "false");
    assert!(!env_bool("_TEST_BOOL_T1", true));

    std::env::set_var("_TEST_BOOL_T2", "0");
    assert!(!env_bool("_TEST_BOOL_T2", true));

    std::env::set_var("_TEST_BOOL_T3", "anything");
    assert!(env_bool("_TEST_BOOL_T3", true));

    // Missing env var returns default (true)
    std::env::remove_var("_TEST_BOOL_T_MISSING");
    assert!(env_bool("_TEST_BOOL_T_MISSING", true));

    // Cleanup
    for k in ["_TEST_BOOL_T1", "_TEST_BOOL_T2", "_TEST_BOOL_T3"] {
        std::env::remove_var(k);
    }
}

// =========================================================================
// Config validation
// =========================================================================

/// Helper to build a minimal valid Config for testing.
fn minimal_config() -> Config {
    Config::test_minimal()
}

#[test]
fn validate_missing_encryption_key_fails() {
    let mut config = minimal_config();
    config.secrets.encryption_key = None;
    let err = config.validate().unwrap_err();
    assert!(err.to_string().contains("SECRETS_ENCRYPTION_KEY"));
}

#[test]
fn validate_encryption_key_present_passes() {
    let config = minimal_config();
    // minimal_config() already has encryption_key set
    assert!(config.validate().is_ok());
}

#[test]
fn auth_provider_defaults_to_local() {
    assert_eq!(AuthProvider::default(), AuthProvider::Local);
    assert!(minimal_config().validate().is_ok());
}

#[test]
fn auth_provider_parses_legacy_no_auth_spellings_as_local() {
    for raw in ["", "disabled", "none", "local", "LOCAL"] {
        assert_eq!(raw.parse::<AuthProvider>().unwrap(), AuthProvider::Local);
    }
}

#[test]
fn auth_provider_rejects_unknown_value() {
    assert!("guthub".parse::<AuthProvider>().is_err());
}

#[test]
fn opaque_browser_sessions_do_not_require_a_signing_secret() {
    let mut config = minimal_config();
    config.auth.session_jwt_secret = None;
    config.sandbox.task_token_secret = None;
    assert!(config.validate().is_ok());
}

#[test]
fn validate_local_accepts_task_token_secret_fallback() {
    let mut config = minimal_config();
    config.auth.session_jwt_secret = None;
    config.sandbox.task_token_secret = Some("fallback-secret".into());
    assert!(config.validate().is_ok());
}

#[test]
fn validate_local_requires_database_url() {
    let mut config = minimal_config();
    config.database.url = None;
    let err = config.validate().unwrap_err();
    assert!(err.to_string().contains("DATABASE_URL"));
}

#[test]
fn validate_local_rejects_short_admin_password() {
    let mut config = minimal_config();
    config.auth.admin_password = Some("short".into());
    let err = config.validate().unwrap_err();
    assert!(err.to_string().contains("ADMIN_PASSWORD"));
}

#[test]
fn validate_local_accepts_long_admin_password() {
    let mut config = minimal_config();
    config.auth.admin_password = Some("a-sufficiently-long-password".into());
    assert!(config.validate().is_ok());
}

#[test]
fn validate_production_with_auth_provider_passes() {
    let mut config = minimal_config();
    config.auth.is_production = true;
    config.auth.provider = AuthProvider::GitHubApp;
    config.auth.github_app_client_id = Some("id".into());
    config.auth.github_app_client_secret = Some("secret".into());
    config.auth.github_app_callback_url = Some("https://example.com/callback".into());
    config.sandbox.task_token_secret = Some("task-secret".into());
    assert!(config.validate().is_ok());
}

#[test]
fn validate_production_without_task_token_secret_fails() {
    // Production must be able to authenticate sandbox gRPC calls; without the
    // signing secret the verification layer would fall back to its dev bypass.
    let mut config = minimal_config();
    config.auth.is_production = true;
    config.auth.provider = AuthProvider::GitHubApp;
    config.auth.github_app_client_id = Some("id".into());
    config.auth.github_app_client_secret = Some("secret".into());
    config.auth.github_app_callback_url = Some("https://example.com/callback".into());
    config.sandbox.task_token_secret = None;
    let err = config.validate().unwrap_err();
    assert!(err.to_string().contains("TASK_TOKEN_SECRET"));
}

#[test]
fn validate_github_app_missing_callback_url_fails() {
    let mut config = minimal_config();
    config.auth.provider = AuthProvider::GitHubApp;
    config.auth.github_app_client_id = Some("id".into());
    config.auth.github_app_client_secret = Some("secret".into());
    config.auth.github_app_callback_url = None;
    let err = config.validate().unwrap_err();
    assert!(err.to_string().contains("GITHUB_APP_CALLBACK_URL"));
}

#[test]
fn validate_github_app_complete_passes() {
    let mut config = minimal_config();
    config.auth.provider = AuthProvider::GitHubApp;
    config.auth.github_app_client_id = Some("id".into());
    config.auth.github_app_client_secret = Some("secret".into());
    config.auth.github_app_callback_url = Some("https://example.com/callback".into());
    assert!(config.validate().is_ok());
}

#[test]
fn validate_auth0_missing_domain_fails() {
    let mut config = minimal_config();
    config.auth.provider = AuthProvider::Auth0;
    config.auth.auth0_domain = None;
    config.auth.auth0_client_id = Some("id".into());
    config.auth.auth0_client_secret = Some("secret".into());
    let err = config.validate().unwrap_err();
    assert!(err.to_string().contains("AUTH0_DOMAIN"));
}

#[test]
fn validate_auth0_complete_passes() {
    let mut config = minimal_config();
    config.auth.provider = AuthProvider::Auth0;
    config.auth.auth0_domain = Some("test.auth0.com".into());
    config.auth.auth0_client_id = Some("id".into());
    config.auth.auth0_client_secret = Some("secret".into());
    assert!(config.validate().is_ok());
}

#[test]
fn validate_oidc_complete_passes() {
    let mut config = minimal_config();
    config.auth.provider = AuthProvider::Oidc;
    config.auth.oidc = Some(OidcConfig {
        provider_id: "company".into(),
        name: "Company SSO".into(),
        issuer: "https://id.example.com/".into(),
        client_id: "vibe".into(),
        client_secret: "secret".into(),
        callback_url: "https://moatless.example.com/api/v1/auth/oidc/company/callback".into(),
        auto_provision: true,
        auto_link_verified_email: true,
        allowed_domains: vec!["example.com".into()],
    });
    assert!(config.validate().is_ok());
}

#[test]
fn validate_oidc_rejects_empty_issuer() {
    let mut config = minimal_config();
    config.auth.provider = AuthProvider::Oidc;
    config.auth.oidc = Some(OidcConfig {
        provider_id: "company".into(),
        name: "Company SSO".into(),
        issuer: String::new(),
        client_id: "vibe".into(),
        client_secret: "secret".into(),
        callback_url: "https://moatless.example.com/api/v1/auth/oidc/company/callback".into(),
        auto_provision: true,
        auto_link_verified_email: true,
        allowed_domains: Vec::new(),
    });
    assert!(config.validate().is_err());
}

#[test]
fn validate_empty_sandbox_namespace_fails() {
    let mut config = minimal_config();
    config.sandbox.namespace = String::new();
    let err = config.validate().unwrap_err();
    assert!(err.to_string().contains("SANDBOX_NAMESPACE"));
}

fn valid_fargate_config() -> Config {
    let mut config = minimal_config();
    config.sandbox.compute_mode = KubernetesComputeMode::Fargate;
    config.sandbox.default_pvc_enabled = false;
    config.sandbox.default_s3_enabled = true;
    config.sandbox.run_as_non_root = true;
    config.sandbox.ephemeral_storage = Some("40Gi".into());
    config.sandbox.sandbox_resources_cpu_request = "4".into();
    config.sandbox.sandbox_resources_cpu_limit = "4".into();
    config.sandbox.sandbox_resources_memory_request = "16Gi".into();
    config.sandbox.sandbox_resources_memory_limit = "16Gi".into();
    config.sandbox.resources_cpu_request = "1000m".into();
    config.sandbox.resources_cpu_limit = "1000m".into();
    config.sandbox.resources_memory_request = "1536Mi".into();
    config.sandbox.resources_memory_limit = "1536Mi".into();
    config
}

#[test]
fn validate_fargate_contract_passes() {
    assert!(valid_fargate_config().validate().is_ok());
}

#[test]
fn validate_fargate_rejects_default_pvc_storage() {
    let mut config = valid_fargate_config();
    config.sandbox.default_pvc_enabled = true;
    let error = config.validate().unwrap_err();
    assert!(error.to_string().contains("DEFAULT_PVC=false"));
}

#[test]
fn validate_fargate_rejects_disabled_s3_storage() {
    let mut config = valid_fargate_config();
    config.sandbox.default_s3_enabled = false;
    let error = config.validate().unwrap_err();
    assert!(error.to_string().contains("DEFAULT_S3=true"));
}

#[test]
fn validate_fargate_rejects_burstable_resources() {
    let mut config = valid_fargate_config();
    config.sandbox.sandbox_resources_cpu_request = "1".into();
    let error = config.validate().unwrap_err();
    assert!(error.to_string().contains("equal sandbox CPU"));
}

// =========================================================================
// AuthConfig Debug redaction
// =========================================================================

#[test]
fn validate_test_api_key_missing_prefix_fails() {
    let mut config = minimal_config();
    config.auth.test_api_key = Some("a".repeat(36));
    let err = config.validate().unwrap_err();
    assert!(err.to_string().contains("TEST_API_KEY"));
    assert!(err.to_string().contains("mvk_"));
}

#[test]
fn validate_test_api_key_too_short_fails() {
    let mut config = minimal_config();
    config.auth.test_api_key = Some("mvk_short".into());
    let err = config.validate().unwrap_err();
    assert!(err.to_string().contains("TEST_API_KEY"));
    assert!(err.to_string().contains("36 characters"));
}

#[test]
fn validate_test_api_key_valid_passes() {
    let mut config = minimal_config();
    config.auth.test_api_key = Some(format!("mvk_{}", "a".repeat(32)));
    assert!(config.validate().is_ok());
}

#[test]
fn auth_config_debug_redacts_secrets() {
    let config = minimal_config();
    let mut auth = config.auth;
    auth.github_app_client_secret = Some("super_secret_123".into());
    auth.auth0_client_secret = Some("another_secret".into());
    auth.test_api_key = Some(format!("mvk_{}", "x".repeat(32)));
    let debug = format!("{auth:?}");
    assert!(!debug.contains("super_secret_123"));
    assert!(!debug.contains("another_secret"));
    assert!(!debug.contains(&"x".repeat(32)));
    assert!(debug.contains("[REDACTED]"));
}

// =========================================================================
// SlackConfig Debug redaction
// =========================================================================

#[test]
fn slack_config_debug_redacts_secrets() {
    let slack = SlackConfig {
        bot_token: Some("xoxb-real-token".into()),
        signing_secret: Some("real-signing-secret".into()),
        bot_user_id: Some("U12345".into()),
    };
    let debug = format!("{slack:?}");
    assert!(!debug.contains("xoxb-real-token"));
    assert!(!debug.contains("real-signing-secret"));
    assert!(debug.contains("[REDACTED]"));
    assert!(debug.contains("U12345")); // bot_user_id should NOT be redacted
}

// =========================================================================
// DecisionConfig
// =========================================================================

fn decision(
    provider: Option<&str>,
    key: Option<&str>,
    base_url: Option<&str>,
    model: Option<&str>,
) -> Result<DecisionConfig, ConfigError> {
    let owned = |value: Option<&str>| value.map(str::to_string);
    DecisionConfig::parse(owned(provider), owned(key), owned(base_url), owned(model))
}

fn jev(config: DecisionConfig) -> JevConfig {
    match config.provider {
        Some(DecisionProviderConfig::Jev(jev)) => jev,
        None => panic!("expected the Jev provider, got none"),
    }
}

#[test]
fn an_unset_or_blank_decision_provider_disables_decisions_even_with_a_key() {
    for provider in [None, Some(""), Some("   ")] {
        let config = decision(provider, Some("key"), None, None).expect("parses");
        assert!(config.provider.is_none(), "{provider:?}");
    }
}

#[test]
fn jev_defaults_to_the_pinned_model_and_public_endpoint() {
    let jev = jev(decision(Some(" JEV "), Some("key"), Some(""), Some("")).expect("parses"));
    assert_eq!(jev.api_key, "key");
    assert_eq!(jev.model, DEFAULT_JEV_MODEL);
    assert_eq!(jev.base_url, "https://api.typesafe.ai");
}

#[test]
fn jev_model_and_base_url_are_overridable() {
    let jev = jev(decision(
        Some("jev"),
        Some("key"),
        Some("http://127.0.0.1:9/"),
        Some("jev-latest"),
    )
    .expect("parses"));
    assert_eq!(jev.model, "jev-latest");
    assert_eq!(jev.base_url, "http://127.0.0.1:9");
}

#[test]
fn jev_without_a_key_refuses_to_boot() {
    for key in [None, Some(" ")] {
        let err = decision(Some("jev"), key, None, None).expect_err("needs a key");
        assert!(err.to_string().contains("TYPESAFE_API_KEY"), "{err}");
    }
}

#[test]
fn an_unknown_decision_provider_refuses_to_boot() {
    let err = decision(Some("oracle"), Some("key"), None, None).expect_err("unknown");
    assert!(
        err.to_string().contains("DECISION_PROVIDER 'oracle'"),
        "{err}"
    );
}

#[test]
fn jev_config_debug_redacts_the_key() {
    let config = decision(Some("jev"), Some("tsk-real-key"), None, None).expect("parses");
    let debug = format!("{config:?}");
    assert!(!debug.contains("tsk-real-key"), "{debug}");
    assert!(debug.contains("[REDACTED]"));
    assert!(debug.contains(DEFAULT_JEV_MODEL));
}

#[test]
fn t3_gate_parser_accepts_enabled_values() {
    for value in ["1", "true", "TRUE", "yes", "on", " on "] {
        assert!(parse_enabled("T3_UI_RPC_ENABLED", value).unwrap());
    }
}

#[test]
fn t3_gate_parser_accepts_disabled_values() {
    for value in ["0", "false", "FALSE", "no", "off", "", "  "] {
        assert!(!parse_enabled("T3_UI_RPC_ENABLED", value).unwrap());
    }
}

#[test]
fn t3_gate_parser_rejects_invalid_values() {
    let error = parse_enabled("T3_UI_RPC_ENABLED", "sometimes").unwrap_err();

    assert!(matches!(
        error,
        ConfigError::Invalid(message) if message == "T3_UI_RPC_ENABLED must be true or false"
    ));
}
