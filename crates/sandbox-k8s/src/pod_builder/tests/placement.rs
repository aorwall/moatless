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
