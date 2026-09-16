//! The node a pod is pinned to, and the affinity a retained placement leaves
//! behind.

use super::fixtures::*;
use super::*;

#[test]
fn a_cache_only_prefers_its_node() {
    let affinity = retained_node_affinity("node-a");
    let node_affinity = affinity.node_affinity.expect("node affinity");

    let preferred = node_affinity
        .preferred_during_scheduling_ignored_during_execution
        .expect("preferred term");
    assert_eq!(
        only_match_field(&preferred[0].preference),
        ("metadata.name", "node-a")
    );
    assert!(
        node_affinity
            .required_during_scheduling_ignored_during_execution
            .is_none()
    );
}

/// The merge is key by key: a global key the row leaves out survives, and one
/// it names is replaced.
#[test]
fn a_row_selector_merges_over_the_global_one() {
    let mut config = test_config();
    config.sandbox.node_selector_json =
        Some(r#"{"workload":"moatless","moatless.io/pool":"spot"}"#.into());
    let mut plan = test_plan();
    plan.repo_config.node_selector = Some(BTreeMap::from([(
        "moatless.io/pool".to_string(),
        "on-demand".to_string(),
    )]));

    let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
    let selector = pod
        .spec
        .as_ref()
        .unwrap()
        .node_selector
        .as_ref()
        .expect("node selector");

    assert_eq!(
        selector.get("workload").map(String::as_str),
        Some("moatless"),
        "a global-only key survives the merge"
    );
    assert_eq!(
        selector.get("moatless.io/pool").map(String::as_str),
        Some("on-demand"),
        "the per-row value wins on a conflicting key"
    );
}

/// An empty per-row selector is not a selector: it leaves the global one as it
/// was rather than writing an empty map the scheduler would ignore.
#[test]
fn an_empty_row_selector_leaves_the_global_one_alone() {
    let mut config = test_config();
    config.sandbox.node_selector_json = Some(r#"{"workload":"moatless"}"#.into());
    let mut plan = test_plan();
    plan.repo_config.node_selector = Some(BTreeMap::new());

    let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
    assert_eq!(
        pod.spec.as_ref().unwrap().node_selector,
        Some(BTreeMap::from([(
            "workload".to_string(),
            "moatless".to_string()
        )]))
    );
}

#[test]
fn a_retained_placement_never_sets_node_name() {
    let mut pod = Pod {
        spec: Some(PodSpec::default()),
        ..Default::default()
    };
    pin_pod_to_node(&mut pod, "node-a").expect("pin");

    let spec = pod.spec.expect("spec");
    assert!(spec.node_name.is_none());
    assert!(spec.affinity.is_some());
}
