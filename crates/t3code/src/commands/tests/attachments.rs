//! What the composer attached, as the sink receives it: decoded bytes, order,
//! uploaded references, and each refusal.

use super::fixtures::*;
use super::*;

/// A one-pixel PNG, as the composer would send it.
fn an_image(name: &str) -> Value {
    json!({
        "type": "image",
        "name": name,
        "mimeType": "image/png",
        "sizeBytes": 70,
        "dataUrl": format!("data:image/png;base64,{PNG_BASE64}")
    })
}

/// The 70 bytes of a 1×1 transparent PNG.
const PNG_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk\
                          YPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

/// A turn carrying `attachments`.
fn turn_with(attachments: Value) -> Value {
    let mut command = turn(None);
    command["message"]["attachments"] = attachments;
    command
}

/// The bytes are what reaches the sink — not the data URL, and not the sizes
/// the client declared, which are only what the client believes.
#[tokio::test]
async fn an_attached_image_reaches_the_sink_decoded() {
    let (result, calls) = run(turn_with(json!([an_image("screenshot.png")]))).await;
    assert!(result.is_ok());
    let Some(Call::SendTurn { attachments, .. }) = calls.first() else {
        panic!("expected a turn, got {calls:?}");
    };
    assert_eq!(
        attachments,
        &vec![Attached::Inline(
            "screenshot.png".to_string(),
            "image/png".to_string(),
            70
        )]
    );
}

/// The order the composer held them in is the order they are stored in, so the
/// bubble renders them the way the person arranged them.
#[tokio::test]
async fn attachments_keep_the_order_they_were_sent_in() {
    let (_, calls) = run(turn_with(json!([an_image("a.png"), an_image("b.png")]))).await;
    let Some(Call::SendTurn { attachments, .. }) = calls.first() else {
        panic!("expected a turn, got {calls:?}");
    };
    assert_eq!(
        attachments,
        &vec![
            Attached::Inline("a.png".to_string(), "image/png".to_string(), 70),
            Attached::Inline("b.png".to_string(), "image/png".to_string(), 70),
        ]
    );
}

/// A composer that has read `attachmentUploads` sends a reference — image or
/// file — carrying no bytes; only the id matters, since the name, type and size
/// beside it are just the client's copy of the upload row.
///
/// An id alone is not provenance, so the resolver measures the stored row
/// against the member kind it was claimed under, which this side must keep
/// naming.
#[tokio::test]
async fn an_uploaded_attachment_reaches_the_sink_as_the_id_it_named() {
    let uploaded = |kind: &str, name: &str, id: &str| {
        json!({
            "type": kind,
            "id": id,
            "name": name,
            "mimeType": if kind == "image" { "image/png" } else { "application/pdf" },
            "sizeBytes": 4096
        })
    };
    let (result, calls) = run(turn_with(json!([
        uploaded("image", "shot.png", "6b1f7b2e-0000-4000-8000-000000000001"),
        uploaded("file", "report.pdf", "6b1f7b2e-0000-4000-8000-000000000002"),
    ])))
    .await;

    assert!(result.is_ok(), "{result:?}");
    let Some(Call::SendTurn { attachments, .. }) = calls.first() else {
        panic!("expected a turn, got {calls:?}");
    };
    assert_eq!(
        attachments,
        &vec![
            Attached::Uploaded(
                "6b1f7b2e-0000-4000-8000-000000000001".to_string(),
                AttachmentKind::Image
            ),
            Attached::Uploaded(
                "6b1f7b2e-0000-4000-8000-000000000002".to_string(),
                AttachmentKind::File
            ),
        ]
    );
}

/// A file has no inline form — only `UploadChatImageAttachment` carries bytes —
/// and an id that is a path is not an id. Both refuse the turn rather than
/// reaching the sink with something it would have to guess at.
#[tokio::test]
async fn a_reference_that_is_not_one_refuses_the_turn() {
    for attachment in [
        // A file with bytes in the command. No client sends this; one that
        // did would be sending a member of a union that does not exist.
        json!({
            "type": "file",
            "name": "notes.txt",
            "mimeType": "text/plain",
            "sizeBytes": 5,
            "dataUrl": "data:text/plain;base64,aGVsbG8="
        }),
        // Neither bytes nor an id.
        json!({ "type": "file", "name": "report.pdf", "mimeType": "application/pdf" }),
        // Outside `ChatAttachmentId`'s alphabet: an id that could leave the
        // route it becomes a segment of.
        json!({ "type": "file", "id": "../secrets", "name": "a.pdf", "mimeType": "application/pdf" }),
        json!({ "type": "image", "id": "", "name": "a.png", "mimeType": "image/png" }),
    ] {
        let (result, calls) = run(turn_with(json!([attachment.clone()]))).await;
        assert!(
            matches!(result, Err(CommandRefusal::Rejected { .. })),
            "expected {attachment} to be rejected, got {result:?}"
        );
        assert!(calls.is_empty(), "expected no write for {attachment}");
    }
}

/// A turn with nothing attached is byte-identical to one sent before this
/// surface read attachments at all.
#[tokio::test]
async fn a_turn_without_attachments_carries_none() {
    let (_, calls) = run(turn(None)).await;
    assert!(matches!(
        calls.first(),
        Some(Call::SendTurn { attachments, .. }) if attachments.is_empty()
    ));
}

/// Each of these is refused rather than dropped: an image that disappears on
/// send reads as Moatless losing it, and nothing reaches the sink, so the
/// refusal is the whole outcome.
#[tokio::test]
async fn an_attachment_that_cannot_be_honoured_refuses_the_turn() {
    let mut oversized = an_image("huge.png");
    oversized["dataUrl"] = json!(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(vec![0u8; MAX_IMAGE_BYTES + 1])
    ));

    let mut not_an_image = an_image("notes.txt");
    not_an_image["dataUrl"] = json!("data:text/plain;base64,aGVsbG8=");

    let mut unreadable = an_image("broken.png");
    unreadable["dataUrl"] = json!("data:image/png;base64,%%%not-base64%%%");

    let mut nameless = an_image("x.png");
    nameless["name"] = json!("   ");

    let mut future_kind = an_image("clip.mp4");
    future_kind["type"] = json!("video");

    for attachment in [
        oversized,
        not_an_image,
        unreadable,
        nameless,
        future_kind,
        json!({ "type": "image", "name": "no-payload.png" }),
    ] {
        let name = attachment["name"].clone();
        let (result, calls) = run(turn_with(json!([attachment]))).await;
        assert!(
            matches!(result, Err(CommandRefusal::Rejected { .. })),
            "expected {name} to be rejected, got {result:?}"
        );
        assert!(calls.is_empty(), "expected no write for {name}");
    }
}

/// The contract caps a turn at eight, and a client that sends more is answering
/// a different contract.
#[tokio::test]
async fn more_attachments_than_the_contract_allows_refuses_the_turn() {
    let too_many: Vec<Value> = (0..=MAX_ATTACHMENTS)
        .map(|index| an_image(&format!("{index}.png")))
        .collect();
    let (result, calls) = run(turn_with(Value::Array(too_many))).await;
    assert!(matches!(result, Err(CommandRefusal::Rejected { .. })));
    assert!(calls.is_empty());
}

/// A data URL that is not base64 has no bytes to read, and a payload with the
/// newlines an encoder wrapped it at does.
#[test]
fn a_data_url_is_read_only_where_it_is_base64() {
    assert!(decode_data_url("data:image/png,not-encoded").is_none());
    assert!(decode_data_url("https://example.com/a.png").is_none());
    assert!(decode_data_url("data:image/png;base64").is_none());

    let (mime_type, bytes) = decode_data_url("data:image/PNG;base64,aGVs\n bG8=").expect("decoded");
    assert_eq!(mime_type, "image/png");
    assert_eq!(bytes, b"hello");
}
