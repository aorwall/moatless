//! T3's server settings: the deployment's own, and the few a viewer saves for
//! themselves through `server.updateSettings`.

use serde_json::{Map, Value, json};
use tracing::warn;

use crate::state::UiRpcState;
use moatless_types::ui_rpc::UiRpcAuthIdentity;

/// The tags whose threads the viewer's lists leave out while
/// [`HIDE_TAGGED_THREADS`] is on.
const HIDDEN_THREAD_TAGS: &str = "hiddenThreadTags";
const HIDE_TAGGED_THREADS: &str = "hideTaggedThreads";

/// The contract's `ThreadTag` length limit.
const MAX_TAG_CHARS: usize = 64;
const MAX_HIDDEN_TAGS: usize = 100;

/// Why a `server.updateSettings` patch is not saved.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PatchRefusal {
    /// Keys this server keeps no value for, sorted.
    Unsupported(Vec<String>),
    /// A key it keeps, with a value it will not store.
    Invalid { key: &'static str, message: String },
}

/// The part of `patch` a viewer may save, normalized, or why none of it is.
///
/// All or nothing: a patch naming any key this server does not keep is
/// refused whole, so a client never reads back half of what it sent.
pub(crate) fn stored_patch(patch: &Map<String, Value>) -> Result<Map<String, Value>, PatchRefusal> {
    let mut unsupported: Vec<String> = patch
        .keys()
        .filter(|key| ![HIDDEN_THREAD_TAGS, HIDE_TAGGED_THREADS].contains(&key.as_str()))
        .cloned()
        .collect();
    if !unsupported.is_empty() {
        unsupported.sort();
        return Err(PatchRefusal::Unsupported(unsupported));
    }
    let mut stored = Map::new();
    if let Some(value) = patch.get(HIDE_TAGGED_THREADS) {
        let Some(on) = value.as_bool() else {
            return Err(invalid(HIDE_TAGGED_THREADS, "must be true or false"));
        };
        stored.insert(HIDE_TAGGED_THREADS.into(), Value::Bool(on));
    }
    if let Some(value) = patch.get(HIDDEN_THREAD_TAGS) {
        stored.insert(HIDDEN_THREAD_TAGS.into(), hidden_tags(value)?);
    }
    Ok(stored)
}

/// Each tag trimmed, as `ThreadTag` decodes one, and the list deduplicated in
/// the order given.
fn hidden_tags(value: &Value) -> Result<Value, PatchRefusal> {
    let Some(entries) = value.as_array() else {
        return Err(invalid(HIDDEN_THREAD_TAGS, "must be a list of tags"));
    };
    let mut tags: Vec<String> = Vec::new();
    for entry in entries {
        let Some(tag) = entry.as_str().map(str::trim) else {
            return Err(invalid(HIDDEN_THREAD_TAGS, "must be a list of tags"));
        };
        if tag.is_empty()
            || tag.chars().count() > MAX_TAG_CHARS
            || tag.chars().any(char::is_control)
        {
            return Err(invalid(
                HIDDEN_THREAD_TAGS,
                format!("`{tag}` is not a tag: one line of 1 to {MAX_TAG_CHARS} characters"),
            ));
        }
        if !tags.iter().any(|kept| kept == tag) {
            tags.push(tag.to_string());
        }
    }
    if tags.len() > MAX_HIDDEN_TAGS {
        return Err(invalid(
            HIDDEN_THREAD_TAGS,
            format!("holds at most {MAX_HIDDEN_TAGS} tags"),
        ));
    }
    Ok(json!(tags))
}

fn invalid(key: &'static str, message: impl Into<String>) -> PatchRefusal {
    PatchRefusal::Invalid {
        key,
        message: message.into(),
    }
}

/// The stored fields this server reports, each of the type the contract
/// decodes; anything else in the row is left out rather than sent.
fn readable(stored: Map<String, Value>) -> Map<String, Value> {
    stored
        .into_iter()
        .filter(|(key, value)| match key.as_str() {
            HIDE_TAGGED_THREADS => value.is_boolean(),
            HIDDEN_THREAD_TAGS => value.is_array(),
            _ => false,
        })
        .collect()
}

/// The viewer's whole settings object: the deployment's, then their own.
///
/// Partial: the client fills every other server setting with its default.
/// Guided reviews are written by a Task the viewer starts. `providerInstances`
/// enables Pi explicitly: T3 resolves a provider's enabled state from settings,
/// and treats an unconfigured Pi as off, which hides it from the picker.
pub(crate) fn settings_object(stored: Map<String, Value>) -> Value {
    let mut settings = json!({
        "reviewGuidesEnabled": true,
        "providerInstances": {
            "pi": { "driver": "pi", "enabled": true },
            "mini-swe": { "driver": "mini-swe", "enabled": true }
        }
    });
    if let Value::Object(fields) = &mut settings {
        fields.extend(readable(stored));
    }
    settings
}

/// [`settings_object`] over the viewer's stored settings. A failed read is
/// logged and reports none of them, rather than failing the config it is in.
pub(crate) async fn viewer_settings(state: &UiRpcState, identity: &UiRpcAuthIdentity) -> Value {
    let stored = match state.viewer_settings.read(&identity.user_id).await {
        Ok(stored) => stored,
        Err(error) => {
            warn!(user_id = %identity.user_id, %error, "T3 viewer settings read failed");
            Map::new()
        }
    };
    settings_object(stored)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(value: Value) -> Result<Map<String, Value>, PatchRefusal> {
        let Value::Object(patch) = value else {
            panic!("a patch is an object")
        };
        stored_patch(&patch)
    }

    #[test]
    fn a_patch_keeps_its_tags_trimmed_and_once() {
        assert_eq!(
            Value::Object(
                patch(json!({
                    "hideTaggedThreads": true,
                    "hiddenThreadTags": [" private", "private", "review"]
                }))
                .unwrap()
            ),
            json!({ "hideTaggedThreads": true, "hiddenThreadTags": ["private", "review"] })
        );
    }

    #[test]
    fn a_patch_naming_another_setting_is_refused_whole() {
        assert_eq!(
            patch(json!({ "hideTaggedThreads": true, "textGenerationModelSelection": {}, "a": 1 })),
            Err(PatchRefusal::Unsupported(vec![
                "a".into(),
                "textGenerationModelSelection".into()
            ]))
        );
    }

    #[test]
    fn a_value_that_is_not_a_tag_list_is_refused() {
        for value in [
            json!({ "hiddenThreadTags": "private" }),
            json!({ "hiddenThreadTags": [1] }),
            json!({ "hiddenThreadTags": ["  "] }),
            json!({ "hiddenThreadTags": ["two\nlines"] }),
            json!({ "hiddenThreadTags": ["x".repeat(65)] }),
            json!({ "hideTaggedThreads": "yes" }),
        ] {
            assert!(
                matches!(patch(value.clone()), Err(PatchRefusal::Invalid { .. })),
                "{value}"
            );
        }
    }

    #[test]
    fn stored_fields_of_the_wrong_type_are_not_reported() {
        let Value::Object(stored) = json!({
            "hideTaggedThreads": "yes",
            "hiddenThreadTags": ["private"],
            "somethingElse": 1
        }) else {
            unreachable!()
        };
        let settings = settings_object(stored);
        assert_eq!(settings["hiddenThreadTags"], json!(["private"]));
        assert!(settings.get("hideTaggedThreads").is_none());
        assert!(settings.get("somethingElse").is_none());
        assert_eq!(settings["reviewGuidesEnabled"], json!(true));
    }
}
