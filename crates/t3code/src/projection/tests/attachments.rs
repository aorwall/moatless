//! What an upload becomes on a message bubble: an image, a file, or nothing the
//! contract has a shape for.

use super::fixtures::*;
use super::*;

/// `ChatImageAttachment` field for field: the id the upload is fetched back by,
/// and the three things the bubble shows beside it.
#[test]
fn an_uploaded_image_becomes_a_chat_attachment() {
    let message = with_files(
        message(1, MessageType::User, "look at this"),
        vec![a_file_context(
            "6b1f7b2e-0000-4000-8000-000000000001",
            "screenshot.png",
            "image/png",
            4096,
        )],
    );

    assert_eq!(
        message_attachments(&message),
        vec![json!({
            "type": "image",
            "id": "6b1f7b2e-0000-4000-8000-000000000001",
            "name": "screenshot.png",
            "mimeType": "image/png",
            "sizeBytes": 4096
        })]
    );
}

/// `ChatFileAttachment` field for field. An upload that is not an image is not
/// dropped: it is the other member of the union, and the bubble shows it as a
/// file the person can download.
#[test]
fn an_uploaded_file_becomes_a_file_attachment() {
    let message = with_files(
        message(1, MessageType::User, "read this"),
        vec![a_file_context(
            "6b1f7b2e-0000-4000-8000-000000000002",
            "report.pdf",
            "application/pdf",
            40 * 1024 * 1024,
        )],
    );

    assert_eq!(
        message_attachments(&message),
        vec![json!({
            "type": "file",
            "id": "6b1f7b2e-0000-4000-8000-000000000002",
            "name": "report.pdf",
            "mimeType": "application/pdf",
            "sizeBytes": 40 * 1024 * 1024
        })]
    );
}

/// The image member is four mime types, not a prefix; an SVG comes back as a
/// file rather than a `ChatImageAttachment` it has no shape for.
///
/// The inline data-URL path accepts any `image/*` too, so this is not only
/// about what the file member lets through.
#[test]
fn an_image_type_the_member_does_not_name_comes_back_as_a_file() {
    for mime in ["image/svg+xml", "image/avif", "image/tiff"] {
        let message = with_files(
            message(1, MessageType::User, "look"),
            vec![a_file_context("id-a", "drawing", mime, 1024)],
        );

        let attachments = message_attachments(&message);
        assert_eq!(attachments.len(), 1, "{mime}: got {attachments:?}");
        assert_eq!(attachments[0]["type"], "file", "{mime}");
        assert_eq!(attachments[0]["mimeType"], mime);
    }

    // The four the contract does name still come back as images.
    for mime in ["image/gif", "image/jpeg", "image/png", "image/webp"] {
        let message = with_files(
            message(1, MessageType::User, "look"),
            vec![a_file_context("id-a", "shot", mime, 1024)],
        );
        assert_eq!(message_attachments(&message)[0]["type"], "image", "{mime}");
    }
}

/// An image type is not by itself the image member.
///
/// The file member allows any type up to 50 MiB, so a large PNG comes back out
/// as the member it fits, not the one its mime type suggests.
#[test]
fn a_large_image_type_comes_back_as_the_file_member() {
    let message = with_files(
        message(1, MessageType::User, "look at this"),
        vec![a_file_context(
            "id-b",
            "huge.png",
            "image/png",
            ATTACHMENT_MAX_IMAGE_BYTES + 1,
        )],
    );

    let attachments = message_attachments(&message);
    assert_eq!(attachments.len(), 1, "got {attachments:?}");
    assert_eq!(attachments[0]["type"], "file");
    assert_eq!(attachments[0]["mimeType"], "image/png");
    assert_eq!(attachments[0]["sizeBytes"], ATTACHMENT_MAX_IMAGE_BYTES + 1);
}

/// The boundary between the two members, for one mime type that could be
/// either. At the image ceiling it is still an image; one byte past it is a
/// file rather than nothing.
#[test]
fn an_image_becomes_a_file_at_the_byte_it_outgrows_the_image_member() {
    let projected = |size| {
        let message = with_files(
            message(1, MessageType::User, "look"),
            vec![a_file_context("id-b", "shot.png", "image/png", size)],
        );
        message_attachments(&message)
            .first()
            .map(|attachment| attachment["type"].as_str().unwrap_or_default().to_string())
    };

    assert_eq!(
        projected(ATTACHMENT_MAX_IMAGE_BYTES).as_deref(),
        Some("image")
    );
    assert_eq!(
        projected(ATTACHMENT_MAX_IMAGE_BYTES + 1).as_deref(),
        Some("file")
    );
    assert_eq!(
        projected(ATTACHMENT_MAX_FILE_BYTES).as_deref(),
        Some("file")
    );
    // Past both members, so there is no shape left to send it in.
    assert_eq!(projected(ATTACHMENT_MAX_FILE_BYTES + 1), None);
}

/// The client decodes the whole snapshot against its schema, so an upload it
/// would refuse costs the reader the conversation, not just the attachment.
///
/// Each of these is left out; the ones beside it are not.
#[test]
fn an_upload_the_contract_has_no_shape_for_is_left_out() {
    let message = with_files(
        message(1, MessageType::User, "look at this"),
        vec![
            // Past `PROVIDER_SEND_TURN_MAX_FILE_BYTES`.
            a_file_context(
                "id-f",
                "huge.zip",
                "application/zip",
                ATTACHMENT_MAX_FILE_BYTES + 1,
            ),
            // `ChatFileAttachment` requires at least one byte; the image
            // member does not, which is why this is the file member's test.
            a_file_context("id-g", "empty.pdf", "application/pdf", 0),
            // Outside `ChatAttachmentId`'s alphabet.
            a_file_context("id c/../d", "ok.png", "image/png", 10),
            // `TrimmedNonEmptyString`.
            a_file_context("id-d", "   ", "image/png", 10),
            a_file_context("id-e", "kept.png", "image/png", 10),
        ],
    );

    let attachments = message_attachments(&message);
    assert_eq!(attachments.len(), 1, "got {attachments:?}");
    assert_eq!(attachments[0]["name"], "kept.png");
}

/// An image of no bytes is still a `ChatImageAttachment`: only the file member
/// sets a minimum, and reading one member's rule onto the other would drop
/// something the client would have rendered.
#[test]
fn an_empty_image_is_still_an_image_attachment() {
    let message = with_files(
        message(1, MessageType::User, "look at this"),
        vec![a_file_context("id-a", "blank.png", "image/png", 0)],
    );

    assert_eq!(message_attachments(&message).len(), 1);
}

/// A name past the contract's limit is cut rather than dropped: the person
/// still sees their image, under a name that is shorter than they gave it.
#[test]
fn an_overlong_name_is_cut_to_what_the_contract_takes() {
    let long = format!("{}.png", "ä".repeat(400));
    let message = with_files(
        message(1, MessageType::User, "look at this"),
        vec![a_file_context("id-a", &long, "image/png", 10)],
    );

    let attachments = message_attachments(&message);
    let name = attachments[0]["name"].as_str().unwrap();
    assert!(
        name.len() <= ATTACHMENT_NAME_MAX_CHARS,
        "{} bytes",
        name.len()
    );
    assert!(long.starts_with(name));
}

/// A Message with no uploads is byte-identical to one projected before this
/// surface read them — `attachments` is an optional key, and an empty array is
/// not the same as its absence.
#[test]
fn a_message_without_uploads_carries_no_attachments_key() {
    let thread = detail(&transcript(
        vec![message(1, MessageType::User, "no files here")],
        Vec::new(),
    ));
    let bubble = &thread["messages"][0];

    assert_eq!(bubble["role"], "user");
    assert!(bubble.get("attachments").is_none());
}

/// Only the person attaches things. An assistant row carrying uploads would be
/// the agent claiming the person sent them.
#[test]
fn only_a_user_bubble_carries_attachments() {
    let thread = detail(&transcript(
        vec![
            with_files(
                message(1, MessageType::User, "look at this"),
                vec![a_file_context("id-a", "a.png", "image/png", 10)],
            ),
            with_files(
                message(2, MessageType::Assistant, "I see it"),
                vec![a_file_context("id-b", "b.png", "image/png", 10)],
            ),
        ],
        Vec::new(),
    ));
    let messages = thread["messages"].as_array().unwrap();

    assert_eq!(messages[0]["attachments"].as_array().unwrap().len(), 1);
    assert!(messages[1].get("attachments").is_none());
}
