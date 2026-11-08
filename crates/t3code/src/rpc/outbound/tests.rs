use super::*;
use serde_json::json;
use std::time::Duration;

fn small_channel(capacity: usize) -> (OutboundSender, mpsc::Receiver<QueuedFrame>) {
    let (mut sender, receiver) = channel(capacity);
    sender.bytes = Arc::new(Semaphore::new(1024));
    (sender, receiver)
}

#[tokio::test]
async fn frames_keep_their_order_and_json_contents() {
    let (sender, mut receiver) = channel(4);
    let frames = [
        json!({"text": "å🦀\n\"", "nested": [null, true, 13]}),
        json!([]),
    ];
    for frame in &frames {
        assert!(sender.send(frame.clone()).await);
    }
    for expected in frames {
        assert_eq!(receiver.recv().await.unwrap().into_value(), expected);
    }
}

#[tokio::test]
async fn a_frame_still_being_written_keeps_its_byte_charge() {
    let (sender, mut receiver) = small_channel(4);
    assert!(sender.send(json!("x".repeat(600))).await);
    let _writing = receiver.recv().await.unwrap();

    assert!(!sender.send(json!("x".repeat(600))).await);
    assert!(sender.disconnected().is_cancelled());
}

#[tokio::test]
async fn completed_writes_release_bytes_for_later_frames() {
    let (sender, mut receiver) = small_channel(4);
    for _ in 0..20 {
        assert!(sender.send(json!("x".repeat(600))).await);
        drop(receiver.recv().await.unwrap());
    }
    assert!(!sender.disconnected().is_cancelled());
}

#[tokio::test]
async fn overload_wakes_producers_waiting_for_a_frame_slot() {
    let (sender, _receiver) = small_channel(1);
    assert!(sender.send(json!("first")).await);
    let waiting = tokio::spawn({
        let sender = sender.clone();
        async move { sender.send(json!("waiting")).await }
    });
    tokio::task::yield_now().await;
    assert!(!waiting.is_finished());

    assert!(!sender.send(json!("x".repeat(1024))).await);
    assert!(
        !tokio::time::timeout(Duration::from_secs(1), waiting)
            .await
            .unwrap()
            .unwrap()
    );
    assert!(!sender.send(json!("later")).await);
}

#[tokio::test]
async fn a_closed_receiver_releases_a_waiting_producer() {
    let (sender, receiver) = channel(1);
    assert!(sender.send(json!(1)).await);
    let waiting = tokio::spawn(async move { sender.send(json!(2)).await });
    tokio::task::yield_now().await;

    drop(receiver);

    assert!(
        !tokio::time::timeout(Duration::from_secs(1), waiting)
            .await
            .unwrap()
            .unwrap()
    );
}

#[tokio::test]
async fn the_frame_limit_counts_json_escapes() {
    let (sender, _receiver) = channel(1);
    let frame = json!("\n".repeat(MAX_FRAME_BYTES / 2));
    assert!(!sender.send(frame).await);
    assert!(sender.disconnected().is_cancelled());
}
