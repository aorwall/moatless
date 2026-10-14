//! The routed method set, read from `methods.tsv`, and what a client is told
//! for a tag that has no handler.

use super::fixtures::*;
use super::*;
use crate::config::OrchestrationProtocol;
use crate::oauth;

/// One row of `methods.tsv`.
pub(crate) struct RoutedMethod {
    pub(crate) tag: &'static str,
    pub(crate) scope: &'static str,
    pub(crate) v1: &'static str,
    pub(crate) v2: &'static str,
}

impl RoutedMethod {
    pub(crate) fn mode(&self, protocol: OrchestrationProtocol) -> &'static str {
        match protocol {
            OrchestrationProtocol::V1 => self.v1,
            OrchestrationProtocol::V2 => self.v2,
        }
    }
}

const MODES: [&str; 5] = ["detached", "session", "stream", "unsupported", "declared"];

pub(crate) fn routed_methods() -> Vec<RoutedMethod> {
    include_str!("methods.tsv")
        .lines()
        .filter(|line| !line.starts_with('#'))
        .map(|line| {
            let columns: Vec<&'static str> = line.split('\t').collect();
            let [tag, scope, v1, v2, v2_handler] = columns[..] else {
                panic!("methods.tsv row has the wrong column count: {line}");
            };
            assert!(
                MODES.contains(&v1) && MODES.contains(&v2),
                "methods.tsv mode: {line}"
            );
            assert!(
                ["own", "same", "-"].contains(&v2_handler),
                "methods.tsv v2_handler: {line}"
            );
            RoutedMethod { tag, scope, v1, v2 }
        })
        .collect()
}

/// A tag with no handler in one protocol is refused for its scope when this
/// deployment withholds it, and is a typed absence otherwise.
#[tokio::test]
async fn a_tag_without_a_handler_answers_for_its_scope_or_its_absence() {
    let state = state();
    for method in routed_methods() {
        for protocol in [OrchestrationProtocol::V1, OrchestrationProtocol::V2] {
            if !matches!(method.mode(protocol), "unsupported" | "declared") {
                continue;
            }
            let mut session = session();
            session.shared.protocol = protocol;
            let output = session
                .handle_json(&request("1", method.tag, json!({})), &state, &viewer())
                .await;
            let error = &output.frames[0]["exit"]["cause"][0]["error"];
            if oauth::GRANTED_SCOPES.contains(&method.scope) {
                assert_eq!(
                    error["_tag"], "UnsupportedMethodError",
                    "{} {protocol:?}",
                    method.tag
                );
                assert_eq!(error["method"], method.tag);
            } else {
                assert_eq!(
                    error["_tag"], "EnvironmentAuthorizationError",
                    "{} {protocol:?}",
                    method.tag
                );
                assert_eq!(error["requiredScope"], method.scope);
            }
        }
    }
}
