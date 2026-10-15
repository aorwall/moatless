//! The method table against the routed set `methods.tsv` snapshots, and the
//! order its session methods answer in.

use super::fixtures::*;
use super::*;
use crate::config::OrchestrationProtocol;
use crate::rpc::table::Handler;
use crate::rpc::table::MethodTable;
use crate::rpc::table::Request;
use serde::de::IgnoredAny;
use std::sync::Mutex;

/// One row of `methods.tsv`.
struct RoutedMethod {
    tag: &'static str,
    scope: &'static str,
    v1: &'static str,
    v2: &'static str,
    v2_handler: &'static str,
}

impl RoutedMethod {
    fn mode(&self, protocol: OrchestrationProtocol) -> &'static str {
        match protocol {
            OrchestrationProtocol::V1 => self.v1,
            OrchestrationProtocol::V2 => self.v2,
        }
    }
}

const MODES: [&str; 5] = ["detached", "session", "stream", "unsupported", "declared"];

fn routed_methods() -> Vec<RoutedMethod> {
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
            RoutedMethod {
                tag,
                scope,
                v1,
                v2,
                v2_handler,
            }
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
            let scope = methods()
                .get(method.tag)
                .expect("every row is in the table")
                .scope;
            if scope.is_granted() {
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

/// Both directions: the table holds every row's tag and no other, and answers
/// each as the row records.
#[test]
fn the_table_routes_exactly_the_snapshotted_set() {
    let snapshot = routed_methods();
    let mut expected: Vec<&str> = snapshot.iter().map(|row| row.tag).collect();
    expected.sort_unstable();
    let tags: Vec<&str> = methods().iter().map(|method| method.tag).collect();
    assert_eq!(tags, expected);

    for row in snapshot {
        let method = methods().get(row.tag).expect("every row is in the table");
        assert_eq!(method.scope.as_str(), row.scope, "{}", row.tag);
        for protocol in [OrchestrationProtocol::V1, OrchestrationProtocol::V2] {
            let mode = method
                .handler(protocol)
                .map_or("unsupported", Handler::mode);
            let recorded = match row.mode(protocol) {
                "declared" => "unsupported",
                recorded => recorded,
            };
            assert_eq!(mode, recorded, "{} {protocol:?}", row.tag);
        }
        match row.v2_handler {
            "own" => assert!(method.v2_is_own(), "{}", row.tag),
            "same" => assert!(!method.v2_is_own(), "{}", row.tag),
            _ => {}
        }
    }
}

/// A session method finishes before anything sent after it in the same frame
/// starts, a detached read included: the read sees the transition done.
#[tokio::test]
async fn an_ordered_transition_waits_for_the_one_before_it() {
    let log: &'static Mutex<Vec<&'static str>> = Box::leak(Box::default());
    let record = |entry: &'static str| log.lock().expect("log").push(entry);
    let table = MethodTable::new()
        .session(
            "test.slowTransition",
            Scope::OrchestrationOperate,
            move |_: Request<IgnoredAny>| async move {
                tokio::time::sleep(Duration::from_millis(30)).await;
                record("slow transition");
                Vec::new()
            },
        )
        .session(
            "test.transition",
            Scope::OrchestrationOperate,
            move |_: Request<IgnoredAny>| async move {
                record("transition");
                Vec::new()
            },
        )
        .detached(
            "test.read",
            Scope::OrchestrationRead,
            move |_: Request<IgnoredAny>| async move {
                record("read");
                Vec::new()
            },
        );
    let (frames, _pushed) = mpsc::channel(OUTBOUND_FRAMES);
    let mut session = ProtocolSession::new(frames, Box::leak(Box::new(table)));
    let batch: Vec<Value> = ["test.slowTransition", "test.transition", "test.read"]
        .iter()
        .enumerate()
        .map(|(id, tag)| {
            json!({ "_tag": "Request", "id": id.to_string(), "tag": tag, "payload": {}, "headers": [] })
        })
        .collect();
    let bytes = serde_json::to_vec(&batch).expect("a batch encodes");

    session.handle_json(&bytes, &state(), &viewer()).await;
    session.detached.wait_until_idle().await;

    assert_eq!(
        *log.lock().expect("log"),
        ["slow transition", "transition", "read"]
    );
}

/// The `Die` a payload answers when its method's declared type does not decode
/// it, from the frame loop or a detached request alike.
fn decode_defect(output: &ProtocolOutput) -> &str {
    let cause = &output.frames[0]["exit"]["cause"][0];
    assert_eq!(cause["_tag"], "Die", "{output:?}");
    cause["defect"].as_str().expect("a defect")
}

#[tokio::test]
async fn a_session_payload_of_the_wrong_type_dies_naming_its_method() {
    let frame = request(
        "1",
        "sandbox.setIdleTimeout",
        json!({ "threadId": "task-1", "minutes": "ten" }),
    );
    let output = session().handle_json(&frame, &state(), &viewer()).await;

    assert!(
        decode_defect(&output).starts_with("sandbox.setIdleTimeout: invalid type: string \"ten\""),
        "{output:?}"
    );
}

#[tokio::test]
async fn a_detached_payload_of_the_wrong_type_dies_naming_its_method() {
    let frame = request(
        "1",
        "pullRequests.list",
        json!({ "state": "open", "limit": "ten" }),
    );
    let output = connection().exchange(&frame, &state(), &viewer()).await;

    assert!(
        decode_defect(&output).starts_with("pullRequests.list: invalid type: string \"ten\""),
        "{output:?}"
    );
}

/// An absent payload decodes as `{}`: a method whose fields are all optional
/// answers it as a request that names none of them.
#[tokio::test]
async fn an_absent_payload_decodes_as_an_empty_one() {
    let frame = serde_json::to_vec(
        &json!({ "_tag": "Request", "id": "1", "tag": "vcs.listRefs", "headers": [] }),
    )
    .expect("a frame encodes");
    let output = connection().exchange(&frame, &state(), &viewer()).await;

    assert_eq!(output.frames[0]["exit"]["_tag"], "Success", "{output:?}");
    assert_eq!(output.frames[0]["exit"]["value"], crate::review::no_refs());
}
