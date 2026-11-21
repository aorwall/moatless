use super::*;
use crate::tests::fixtures::{state, viewer};
use axum::extract::ws::WebSocketUpgrade;
use axum::routing::get;
use moatless_ports::task::MockTaskReader;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::task::JoinSet;

async fn send_request(socket: &mut TcpStream, id: &str, tag: &str) {
    let payload = serde_json::to_vec(&json!({
        "_tag": "Request", "id": id, "tag": tag, "payload": {}, "headers": []
    }))
    .unwrap();
    assert!(payload.len() < 126);
    let mut frame = vec![0x81, 0x80 | payload.len() as u8, 0, 0, 0, 0];
    frame.extend(payload);
    socket.write_all(&frame).await.unwrap();
}

async fn receive_frame(socket: &mut TcpStream) -> Value {
    let mut header = [0; 2];
    socket.read_exact(&mut header).await.unwrap();
    assert_eq!(header[0], 0x81);
    let length = match header[1] {
        126 => usize::from(socket.read_u16().await.unwrap()),
        127 => socket.read_u64().await.unwrap() as usize,
        length => usize::from(length),
    };
    let mut payload = vec![0; length];
    socket.read_exact(&mut payload).await.unwrap();
    serde_json::from_slice(&payload).unwrap()
}

#[tokio::test]
async fn overload_ends_a_connection_even_while_its_socket_write_is_blocked() {
    let mut state = state();
    let subscribed = Arc::new(tokio::sync::Notify::new());
    let mut tasks = MockTaskReader::new();
    tasks.expect_may_access().returning({
        let subscribed = subscribed.clone();
        move |_, _| {
            subscribed.notify_one();
            Ok(true)
        }
    });
    state.preview.tasks = Arc::new(tasks);
    let tabs = state.preview.tabs.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (closed, returned) = oneshot::channel();
    let closed = Arc::new(std::sync::Mutex::new(Some(closed)));
    let router = axum::Router::new().route(
        "/ws",
        get(move |upgrade: WebSocketUpgrade| {
            let state = state.clone();
            let closed = closed.clone();
            async move {
                upgrade.on_upgrade(move |socket| async move {
                    serve_websocket(socket, state, viewer(), OrchestrationProtocol::V1).await;
                    closed.lock().unwrap().take().unwrap().send(()).unwrap();
                })
            }
        }),
    );
    let mut servers = JoinSet::new();
    servers.spawn(async move { axum::serve(listener, router).await.unwrap() });
    let mut socket = TcpStream::connect(address).await.unwrap();
    socket.write_all(b"GET /ws HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n").await.unwrap();
    let mut headers = Vec::new();
    while !headers.ends_with(b"\r\n\r\n") {
        headers.push(socket.read_u8().await.unwrap());
    }
    assert!(headers.starts_with(b"HTTP/1.1 101"));
    send_request(&mut socket, "1", "subscribePreviewEvents").await;
    send_request(&mut socket, "2", "server.probe").await;
    assert_eq!(receive_frame(&mut socket).await["requestId"], "2");
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut retry = tokio::time::interval(Duration::from_millis(10));
        loop {
            tokio::select! {
                () = subscribed.notified() => break,
                _ = retry.tick() => tabs.publish(json!({"threadId": "task-1"})),
            }
        }
    })
    .await
    .expect("the preview producer subscribed");

    for _ in 0..8 {
        tabs.publish(json!({"threadId": "task-1", "payload": "x".repeat(8 * 1024 * 1024)}));
    }

    tokio::time::timeout(Duration::from_secs(10), returned)
        .await
        .expect("overload must interrupt the blocked socket write")
        .unwrap();
}

#[tokio::test]
async fn a_frame_being_answered_still_answers_the_heartbeat_and_holds_the_rest_in_order() {
    let (client, mut incoming) = futures::channel::mpsc::unbounded();
    let (mut sink, mut replies) = futures::channel::mpsc::unbounded::<Message>();
    let (finish, finished) = oneshot::channel::<()>();
    let handling = async move {
        finished.await.unwrap();
        ProtocolOutput {
            frames: vec![json!({"answered": true})],
            eof: false,
        }
    };
    let mut held = VecDeque::new();
    let (_frames, mut pushed) = outbound::channel(OUTBOUND_FRAMES);
    let (mut deferred, opened_stream) = (VecDeque::new(), AtomicBool::new(false));
    let pushing = Pushing {
        pushed: &mut pushed,
        deferred: &mut deferred,
        opened_stream: &opened_stream,
    };
    let driving = async {
        client
            .unbounded_send(Ok(Message::Text(r#"{"_tag":"Ping"}"#.into())))
            .unwrap();
        assert_eq!(replies.next().await, Some(Message::Text(PONG.into())));
        client
            .unbounded_send(Ok(Message::Text(r#"{"_tag":"Request","id":"7"}"#.into())))
            .unwrap();
        client
            .unbounded_send(Ok(Message::Ping(vec![1].into())))
            .unwrap();
        assert_eq!(replies.next().await, Some(Message::Pong(vec![1].into())));
        client
            .unbounded_send(Ok(Message::Text(r#"[{"_tag":"Ping"}]"#.into())))
            .unwrap();
        assert_eq!(replies.next().await, Some(Message::Text(PONG.into())));
        finish.send(()).unwrap();
    };

    let (output, ()) = tokio::join!(
        keeping_alive(handling, &mut incoming, &mut sink, &mut held, pushing),
        driving
    );

    assert_eq!(output.frames, vec![json!({"answered": true})]);
    let held: Vec<_> = held
        .into_iter()
        .map(|message| message.unwrap().unwrap())
        .collect();
    assert_eq!(
        held,
        vec![Message::Text(r#"{"_tag":"Request","id":"7"}"#.into())]
    );
}

#[test]
fn only_a_frame_of_nothing_but_pings_is_answered_out_of_order() {
    assert!(only_pings(br#"{"_tag":"Ping"}"#));
    assert!(only_pings(br#"[{"_tag":"Ping"},{"_tag":"Ping"}]"#));
    assert!(!only_pings(
        br#"[{"_tag":"Ping"},{"_tag":"Request","id":"1"}]"#
    ));
    assert!(!only_pings(b"[]"));
    assert!(!only_pings(b"not json"));
}

#[tokio::test]
async fn a_full_hold_stops_reading_so_the_socket_backs_up() {
    let (client, mut incoming) = futures::channel::mpsc::unbounded();
    let (mut sink, _replies) = futures::channel::mpsc::unbounded::<Message>();
    for id in 0..MAX_HELD + 10 {
        client
            .unbounded_send(Ok(Message::Text(format!(r#"{{"id":"{id}"}}"#).into())))
            .unwrap();
    }
    let (finish, finished) = oneshot::channel::<()>();
    let handling = async move {
        finished.await.unwrap();
        ProtocolOutput::default()
    };
    let mut held = VecDeque::new();
    let (_frames, mut pushed) = outbound::channel(OUTBOUND_FRAMES);
    let (mut deferred, opened_stream) = (VecDeque::new(), AtomicBool::new(false));
    let pushing = Pushing {
        pushed: &mut pushed,
        deferred: &mut deferred,
        opened_stream: &opened_stream,
    };
    let releasing = async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        finish.send(()).unwrap();
    };

    tokio::join!(
        keeping_alive(handling, &mut incoming, &mut sink, &mut held, pushing),
        releasing
    );

    assert_eq!(held.len(), MAX_HELD);
    assert_eq!(incoming.size_hint().0, 10);
}

#[tokio::test]
async fn a_frame_being_answered_still_writes_what_subscriptions_push() {
    let (_client, mut incoming) = futures::channel::mpsc::unbounded();
    let (mut sink, mut replies) = futures::channel::mpsc::unbounded::<Message>();
    let (frames, mut pushed) = outbound::channel(OUTBOUND_FRAMES);
    let (finish, finished) = oneshot::channel::<()>();
    let handling = async move {
        finished.await.unwrap();
        ProtocolOutput::default()
    };
    let mut held = VecDeque::new();
    let (mut deferred, opened_stream) = (VecDeque::new(), AtomicBool::new(false));
    let pushing = Pushing {
        pushed: &mut pushed,
        deferred: &mut deferred,
        opened_stream: &opened_stream,
    };
    let driving = async {
        assert!(frames.send(json!({"pushed": 1})).await);
        assert_eq!(
            replies.next().await,
            Some(Message::Text(r#"{"pushed":1}"#.into()))
        );
        finish.send(()).unwrap();
    };

    tokio::join!(
        keeping_alive(handling, &mut incoming, &mut sink, &mut held, pushing),
        driving
    );

    assert!(deferred.is_empty());
}

#[tokio::test]
async fn a_frame_pushed_after_a_stream_opens_waits_for_the_answer_that_opened_it() {
    let (_client, mut incoming) = futures::channel::mpsc::unbounded();
    let (mut sink, mut replies) = futures::channel::mpsc::unbounded::<Message>();
    let (frames, mut pushed) = outbound::channel(OUTBOUND_FRAMES);
    let (finish, finished) = oneshot::channel::<()>();
    let handling = async move {
        finished.await.unwrap();
        ProtocolOutput::default()
    };
    let mut held = VecDeque::new();
    let (mut deferred, opened_stream) = (VecDeque::new(), AtomicBool::new(true));
    let pushing = Pushing {
        pushed: &mut pushed,
        deferred: &mut deferred,
        opened_stream: &opened_stream,
    };
    let driving = async {
        assert!(frames.send(json!({"pushed": 1})).await);
        tokio::time::sleep(Duration::from_millis(20)).await;
        finish.send(()).unwrap();
    };

    tokio::join!(
        keeping_alive(handling, &mut incoming, &mut sink, &mut held, pushing),
        driving
    );

    assert!(replies.try_recv().is_err(), "nothing was written yet");
    assert_eq!(deferred.len(), 1);
}
