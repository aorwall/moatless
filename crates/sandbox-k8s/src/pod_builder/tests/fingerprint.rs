//! The fingerprint that decides whether a pod needs rebuilding, and the
//! generation stamped alongside it.

use super::fixtures::*;
use super::*;

#[test]
fn pod_template_carries_operation_generation() {
    let mut pod = Pod::default();
    stamp_operation_generation(
        &mut pod,
        RuntimeOperationContext::new(moatless_sandbox_runtime::operation::OperationGeneration(31)),
    );
    assert_eq!(
        pod.metadata
            .labels
            .as_ref()
            .and_then(|labels| labels.get(OPERATION_GENERATION_LABEL))
            .map(String::as_str),
        Some("31")
    );
}

/// `ProvisionPlan::env_vars` is a `HashMap`, so the env order changes between
/// builds of one config. An order-sensitive hash would rebuild every sandbox on
/// every start.
#[test]
fn pod_spec_fingerprint_ignores_env_order() {
    let one = pod_with_env(vec![env_var("A", "1"), env_var("B", "2")]);
    let other = pod_with_env(vec![env_var("B", "2"), env_var("A", "1")]);

    assert_eq!(pod_spec_fingerprint(&one), pod_spec_fingerprint(&other));
}

/// The whole round trip an unchanged config takes: a create stamps the hash on
/// a pod built with minted tokens, `build_deployment_object` copies it onto the
/// template, and a later start compares it against a pod built without tokens.
///
/// All three steps have to agree, or every sandbox rebuilds on every start.
#[test]
fn a_stamped_template_matches_a_later_build_of_the_same_config() {
    let config = test_config();
    let plan = test_plan();
    let created = build_pod(
        &config,
        "task-1",
        &plan,
        None,
        Some(SandboxAuthTokens {
            bootstrap: "bootstrap-token",
            legacy_user_token: Some("user-token"),
        }),
    )
    .unwrap();
    let dep = crate::deployment::build_deployment_object("moatless", "task-1", created).unwrap();

    let compared = build_pod(&config, "task-1", &plan, None, None).unwrap();

    assert!(crate::deployment::template_matches_spec(
        &dep,
        &pod_spec_fingerprint(&compared)
    ));
}
