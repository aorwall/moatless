//! A request id returns in the JSON type it arrived as, on every frame that
//! carries one.

use super::fixtures::*;
use super::*;

#[tokio::test]
async fn a_request_id_returns_in_the_json_type_it_arrived_as() {
    // The client resolves a response by looking `requestId` up in a map
    // keyed by the value it sent, and coerces nothing. Answering `21` with
    // `"21"` matches no caller, so the request never completes and the
    // connection attempt times out with every frame looking correct.
    for id in [json!(21), json!("21")] {
        let frame = serde_json::to_vec(&json!({
            "_tag": "Request",
            "id": id,
            "tag": "server.getConfig",
            "payload": {},
            "headers": []
        }))
        .unwrap();
        let mut connection = connection();
        let output = connection.exchange(&frame, &state(), &viewer()).await;
        assert_eq!(output.frames[0]["_tag"], "Exit");
        assert_eq!(output.frames[0]["exit"]["_tag"], "Success");
        assert_eq!(
            output.frames[0]["requestId"], id,
            "a {id} request id must come back as {id}, not re-typed"
        );
    }
}

#[tokio::test]
async fn a_numeric_stream_id_returns_numeric_on_every_frame() {
    let frame = serde_json::to_vec(&json!({
        "_tag": "Request",
        "id": 4,
        "tag": "subscribeServerConfig",
        "payload": {},
        "headers": []
    }))
    .unwrap();
    let mut session = session();
    let output = session.handle_json(&frame, &state(), &viewer()).await;
    assert_eq!(output.frames[0]["_tag"], "Chunk");
    assert_eq!(
        output.frames[0]["requestId"],
        json!(4),
        "a stream's chunks correlate by the same id its request carried"
    );

    let interrupt = serde_json::to_vec(&json!({ "_tag": "Interrupt", "requestId": 4 })).unwrap();
    let output = session.handle_json(&interrupt, &state(), &viewer()).await;
    assert_eq!(output.frames[0]["_tag"], "Exit");
    assert_eq!(output.frames[0]["requestId"], json!(4));
}

#[tokio::test]
async fn a_numeric_id_is_accepted_on_every_frame_that_carries_one() {
    for frame in [
        json!({ "_tag": "Ack", "requestId": 7 }),
        json!({ "_tag": "Interrupt", "requestId": 7 }),
    ] {
        let tag = frame["_tag"].as_str().unwrap().to_string();
        let mut session = session();
        let output = session
            .handle_json(&serde_json::to_vec(&frame).unwrap(), &state(), &viewer())
            .await;
        let defected = output.frames.iter().any(|frame| frame["_tag"] == "Defect");
        assert!(
            !defected,
            "{tag} with a numeric id should parse, got {output:?}"
        );
    }
}

#[tokio::test]
async fn a_fractional_request_id_is_refused() {
    let frame = serde_json::to_vec(&json!({
        "_tag": "Request",
        "id": 21.5,
        "tag": "server.getConfig",
        "payload": {},
        "headers": []
    }))
    .unwrap();
    let mut session = session();
    let output = session.handle_json(&frame, &state(), &viewer()).await;
    assert_eq!(output.frames[0]["_tag"], "Defect");
}

#[tokio::test]
async fn ping_returns_effect_rpc_pong() {
    let mut session = session();
    let output = session
        .handle_json(br#"{"_tag":"Ping"}"#, &state(), &viewer())
        .await;
    assert_eq!(output.frames, vec![json!({ "_tag": "Pong" })]);
}

#[tokio::test]
async fn a_request_id_can_be_reused_after_its_producer_finishes() {
    let mut session = session();
    session
        .streams
        .start_produced(&request_id("3"), None, |_| async { Ok(()) });
    session.streams.wait_until_finished("3").await;

    let output = session
        .handle_json(
            &request("3", "server.probe", json!({})),
            &state(),
            &viewer(),
        )
        .await;

    assert_eq!(output.frames[0]["_tag"], "Exit");
    assert_eq!(output.frames[0]["exit"]["_tag"], "Success");
}

#[tokio::test]
async fn a_json_array_is_one_effect_rpc_batch() {
    let mut session = session();
    let output = session.handle_json(
        br#"[{"_tag":"Ping"},{"_tag":"Request","id":"4","tag":"server.probe","payload":{},"headers":[]}]"#,
        &state(),
        &viewer(),
    ).await;
    assert_eq!(output.frames.len(), 2);
    assert_eq!(output.frames[0]["_tag"], "Pong");
    assert_eq!(output.frames[1]["_tag"], "Exit");
}
