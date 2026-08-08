//! Conversions between API `MessageContextInput` and DB `TaskMessageContext` rows.

use super::models::TaskMessageContext;
use crate::task::schemas::{
    ElementContextData, FileContextData, MessageContextInput, PreviewUrlContextData,
};

const TYPE_ELEMENT: &str = "element";
const TYPE_PREVIEW_URL: &str = "previewUrl";
const TYPE_FILE: &str = "file";

/// Convert an API input variant into a DB row ready to persist.
///
/// `task_message_id` is filled in by the DAO at insert time, so we set 0 here.
pub fn message_context_input_to_row(ctx: &MessageContextInput) -> TaskMessageContext {
    let (context_type, reference_id, data) = match ctx {
        MessageContextInput::Element { reference_id, data } => (
            TYPE_ELEMENT,
            reference_id.clone(),
            serde_json::to_value(data).unwrap_or(serde_json::Value::Null),
        ),
        MessageContextInput::PreviewUrl { reference_id, data } => (
            TYPE_PREVIEW_URL,
            reference_id.clone(),
            serde_json::to_value(data).unwrap_or(serde_json::Value::Null),
        ),
        MessageContextInput::File { reference_id, data } => (
            TYPE_FILE,
            reference_id.clone(),
            serde_json::to_value(data).unwrap_or(serde_json::Value::Null),
        ),
    };

    TaskMessageContext {
        id: 0,
        task_message_id: 0,
        context_type: context_type.to_string(),
        reference_id,
        data,
        created_at: None,
    }
}

/// Convert a DB row back into an API output variant.
///
/// Returns `None` if the row's `type`/`data` shape is not recognised so the
/// transform layer can drop forward-incompatible rows without failing.
pub fn task_message_context_to_input(row: &TaskMessageContext) -> Option<MessageContextInput> {
    match row.context_type.as_str() {
        TYPE_ELEMENT => {
            let data: ElementContextData = serde_json::from_value(row.data.clone()).ok()?;
            Some(MessageContextInput::Element {
                reference_id: row.reference_id.clone(),
                data,
            })
        }
        TYPE_PREVIEW_URL => {
            let data: PreviewUrlContextData = serde_json::from_value(row.data.clone()).ok()?;
            Some(MessageContextInput::PreviewUrl {
                reference_id: row.reference_id.clone(),
                data,
            })
        }
        TYPE_FILE => {
            let data: FileContextData =
                serde_json::from_value(row.data.clone()).unwrap_or_default();
            Some(MessageContextInput::File {
                reference_id: row.reference_id.clone(),
                data,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_file_context() {
        let input = MessageContextInput::File {
            reference_id: "upload-1".to_string(),
            data: FileContextData {
                filename: "report.pdf".to_string(),
                mime_type: "application/pdf".to_string(),
                size: 1234,
            },
        };

        let row = message_context_input_to_row(&input);
        assert_eq!(row.context_type, "file");
        assert_eq!(row.reference_id, "upload-1");

        let back = task_message_context_to_input(&row).expect("convert back");
        match back {
            MessageContextInput::File { reference_id, data } => {
                assert_eq!(reference_id, "upload-1");
                assert_eq!(data.filename, "report.pdf");
                assert_eq!(data.mime_type, "application/pdf");
                assert_eq!(data.size, 1234);
            }
            _ => panic!("Expected File variant"),
        }
    }

    #[test]
    fn round_trip_preview_url_context() {
        let input = MessageContextInput::PreviewUrl {
            reference_id: "preview".to_string(),
            data: PreviewUrlContextData {
                path: "/dashboard".to_string(),
                url: Some("https://example.com/dashboard".to_string()),
            },
        };

        let row = message_context_input_to_row(&input);
        assert_eq!(row.context_type, "previewUrl");

        let back = task_message_context_to_input(&row).expect("convert back");
        match back {
            MessageContextInput::PreviewUrl { data, .. } => {
                assert_eq!(data.path, "/dashboard");
                assert_eq!(data.url.as_deref(), Some("https://example.com/dashboard"));
            }
            _ => panic!("Expected PreviewUrl variant"),
        }
    }

    #[test]
    fn unknown_context_type_returns_none() {
        let row = TaskMessageContext {
            id: 1,
            task_message_id: 1,
            context_type: "future_kind".to_string(),
            reference_id: "ref".to_string(),
            data: serde_json::json!({}),
            created_at: None,
        };
        assert!(task_message_context_to_input(&row).is_none());
    }
}
