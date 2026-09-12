//! What a per-repo service account, network profile or egress opt-in changes
//! on the pod.

use super::fixtures::*;
use super::*;

/// Per-repo `sandboxServiceAccount` overrides the global default SA, and
/// falls back to the global default (here `None`) when unset.
#[test]
fn per_repo_sandbox_service_account_overrides_global() {
    // Unset on the repo → falls back to the global default (None in test_config).
    let config = test_config();
    let plan = test_plan();
    let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
    assert_eq!(
        pod.spec.as_ref().unwrap().service_account_name,
        None,
        "unset per-repo SA falls back to the global default"
    );

    // Set on the repo → wins even when a global default is configured.
    let mut config = test_config();
    config.sandbox.service_account_name = Some("global-default-sa".to_string());
    let mut plan = test_plan();
    plan.repo_config.sandbox_service_account = Some("moatless-dev".to_string());
    let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
    assert_eq!(
        pod.spec.as_ref().unwrap().service_account_name.as_deref(),
        Some("moatless-dev"),
        "per-repo SA overrides the global default"
    );
}

/// Per-repo `networkProfile` adds the `moatless.io/network-profile` pod label
/// (which a static `NetworkPolicy` selects); unset/blank adds no label.
#[test]
fn per_repo_network_profile_sets_pod_label() {
    const KEY: &str = "moatless.io/network-profile";

    // Unset → no profile label.
    let config = test_config();
    let plan = test_plan();
    let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
    assert!(
        !pod.metadata.labels.as_ref().unwrap().contains_key(KEY),
        "unset network_profile adds no label"
    );

    // Set → label carries the value.
    let mut plan = test_plan();
    plan.repo_config.network_profile = Some("sealed-egress".to_string());
    let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
    assert_eq!(
        pod.metadata
            .labels
            .as_ref()
            .unwrap()
            .get(KEY)
            .map(String::as_str),
        Some("sealed-egress"),
        "network_profile sets the pod label a NetworkPolicy can select"
    );

    // Blank/whitespace → treated as unset (no label, avoids invalid label value).
    let mut plan = test_plan();
    plan.repo_config.network_profile = Some("   ".to_string());
    let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
    assert!(
        !pod.metadata.labels.as_ref().unwrap().contains_key(KEY),
        "blank network_profile adds no label"
    );
}

/// The egress proxy env is injected only when a URL is configured AND the
/// pod's network profile is in the opt-in list.
#[test]
fn egress_proxy_env_injected_only_for_opted_in_profile() {
    const URL: &str = "http://moatless-vibe-egress-proxy.moatless.svc.cluster.local:3128";

    let sandbox_proxy = |pod: &Pod, key: &str| -> Option<String> {
        pod.spec
            .as_ref()
            .unwrap()
            .containers
            .iter()
            .find(|c| c.name == "sandbox")
            .unwrap()
            .env
            .as_ref()
            .unwrap()
            .iter()
            .find(|e| e.name == key)
            .and_then(|e| e.value.clone())
    };

    // Proxy configured + profile opted in → HTTPS_PROXY + NO_PROXY set.
    let mut config = test_config();
    config.sandbox.egress_proxy_url = Some(URL.to_string());
    config.sandbox.egress_proxy_profiles = vec!["sealed-egress".to_string()];
    config.sandbox.egress_proxy_no_proxy = "localhost,.svc.cluster.local".to_string();
    let mut plan = test_plan();
    plan.repo_config.network_profile = Some("sealed-egress".to_string());
    let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
    assert_eq!(sandbox_proxy(&pod, "HTTPS_PROXY").as_deref(), Some(URL));
    assert_eq!(sandbox_proxy(&pod, "http_proxy").as_deref(), Some(URL));
    assert_eq!(
        sandbox_proxy(&pod, "NO_PROXY").as_deref(),
        Some("localhost,.svc.cluster.local")
    );

    // Profile NOT in the opt-in list → no proxy env, even with a URL set.
    let mut plan = test_plan();
    plan.repo_config.network_profile = Some("some-other-profile".to_string());
    let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
    assert!(sandbox_proxy(&pod, "HTTPS_PROXY").is_none());

    // No proxy URL configured (default) → no proxy env, even when opted in.
    let config = test_config();
    let mut plan = test_plan();
    plan.repo_config.network_profile = Some("sealed-egress".to_string());
    let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
    assert!(sandbox_proxy(&pod, "HTTPS_PROXY").is_none());
}
