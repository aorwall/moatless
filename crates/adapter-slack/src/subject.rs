//! Canonical Slack subject coordinates.
//!
//! Every Slack destination — a Binding's thread properties, a `channel:`/`dm:`/
//! `thread:` string, a reaction target — is read here and nowhere else.

use moatless_loops::adapters::error::{AdapterError, SharedAdapterError};
use moatless_types::adapter::BindingContext;
use moatless_loops::adapters::{AdapterDestinationBuilder, BindingDestination};

use super::adapter::SlackAdapter;

const SUBJECT_KIND: &str = "slack_thread";

/// The Slack coordinates a thread-properties bag names.
///
/// `AdapterDestinationBuilder` takes an untyped `serde_json::Value` and keeps
/// doing so (spec Open question 3), so the bag is read exactly once — here —
/// and every caller inside the Adapter goes through these fields instead of
/// reaching into the JSON again with its own defaults.
struct SlackThread {
    /// Workspace id. `"default"` when the event carried none, which is the
    /// value the subject key has always been built from.
    team: String,
    channel: String,
    thread_ts: String,
    /// The message's own timestamp, which for a top-level post equals
    /// `thread_ts`.
    ts: String,
}

impl SlackThread {
    /// Read the bag, falling back to the event's own coordinates per field.
    fn from_thread_properties(
        properties: &serde_json::Value,
        source_id: &str,
        external_thread_id: &str,
    ) -> Self {
        let channel = properties
            .get("channel")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(source_id);
        let thread_ts = properties
            .get("thread_ts")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(external_thread_id);
        let team = properties
            .get("team_id")
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or("default");
        let ts = properties
            .get("ts")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(thread_ts);

        Self {
            team: team.to_string(),
            channel: channel.to_string(),
            thread_ts: thread_ts.to_string(),
            ts: ts.to_string(),
        }
    }

    fn subject_key(&self) -> String {
        format!("{}:{}:{}", self.team, self.channel, self.thread_ts)
    }

    fn destination_config(&self, filter_user_id: Option<&str>) -> serde_json::Value {
        let mut config = serde_json::json!({
            "channel": self.channel,
            "thread_ts": self.thread_ts,
            "ts": self.ts,
        });
        if let Some(filter_user_id) = filter_user_id {
            config["filter_user_id"] = serde_json::json!(filter_user_id);
        }
        config
    }

    fn binding_destination(&self, filter_user_id: Option<&str>) -> BindingDestination {
        BindingDestination::new(
            SUBJECT_KIND,
            self.subject_key(),
            self.destination_config(filter_user_id),
        )
    }
}

/// The thread coordinates a Binding's `destination_config` carries.
///
/// Each field is handed back exactly as it was found — absent stays absent,
/// empty stays empty — because the three callers disagree about what a missing
/// one means: a reply errors, a context fetch returns nothing, a fallback
/// `LoopContext` takes the empty string. Reading the bag is shared; deciding
/// what an absence costs stays at the call site.
pub(super) struct ThreadCoordinates<'a> {
    pub(super) channel: Option<&'a str>,
    pub(super) thread_ts: Option<&'a str>,
}

/// Read `channel` and `thread_ts` out of a Binding's `destination_config`.
pub(super) fn thread_coordinates(destination_config: &serde_json::Value) -> ThreadCoordinates<'_> {
    ThreadCoordinates {
        channel: destination_config
            .get("channel")
            .and_then(serde_json::Value::as_str),
        thread_ts: destination_config
            .get("thread_ts")
            .and_then(serde_json::Value::as_str),
    }
}

/// Parse a Slack destination string into `(channel, optional thread_ts)`.
///
/// Accepted shapes:
/// - `channel:<id>` — top-level post to `<id>`
/// - `dm:<id>` — top-level post to DM channel `<id>` (`D…`)
/// - `thread:<channel>/<thread_ts>` — reply in existing thread
/// - raw `<id>` (no prefix) — treated as a channel ID; matches the legacy
///   behavior before the prefix syntax was introduced.
pub(super) fn parse_destination(
    destination: &str,
) -> Result<(String, Option<String>), AdapterError> {
    let raw = destination.trim();
    if raw.is_empty() {
        return Err(
            SharedAdapterError::BadRequest("Slack destination must not be empty".into()).into(),
        );
    }
    if let Some(rest) = raw.strip_prefix("thread:") {
        let (channel, thread_ts) = rest.split_once('/').ok_or_else(|| {
            SharedAdapterError::BadRequest(
                "Slack thread destination must be `thread:<channel>/<thread_ts>`".into(),
            )
        })?;
        if channel.is_empty() || thread_ts.is_empty() {
            return Err(SharedAdapterError::BadRequest(
                "Slack thread destination requires both channel and thread_ts".into(),
            )
            .into());
        }
        return Ok((channel.to_string(), Some(thread_ts.to_string())));
    }
    if let Some(rest) = raw.strip_prefix("channel:") {
        if rest.is_empty() {
            return Err(SharedAdapterError::BadRequest(
                "Slack channel destination must not be empty".into(),
            )
            .into());
        }
        return Ok((rest.to_string(), None));
    }
    if let Some(rest) = raw.strip_prefix("dm:") {
        if rest.is_empty() {
            return Err(SharedAdapterError::BadRequest(
                "Slack DM destination must not be empty".into(),
            )
            .into());
        }
        return Ok((rest.to_string(), None));
    }
    // Raw fallback — accept a bare channel ID for backward compatibility.
    Ok((raw.to_string(), None))
}

/// Resolve the `(channel, ts)` pair needed by Slack's reactions API from the
/// target message's external id, falling back to the binding's
/// `destination_config` for `channel`.
pub(super) fn reaction_target<'a>(
    binding: &'a BindingContext,
    target_external_id: &'a str,
) -> Result<(String, &'a str), AdapterError> {
    let ts = super::external_id::ts_from_message_id(target_external_id).ok_or_else(|| {
        SharedAdapterError::BadRequest(format!(
            "Slack reaction target `{target_external_id}` is not a valid slack:ts:<ts> id"
        ))
    })?;
    let channel = thread_coordinates(&binding.destination_config)
        .channel
        .ok_or_else(|| {
            SharedAdapterError::BadRequest(
                "Slack binding destination_config has no `channel` field".into(),
            )
        })?
        .to_string();
    Ok((channel, ts))
}

/// The `(subject_kind, subject_key, destination_config)` a direct post to
/// `channel` under `thread_ts` binds to.
pub(super) fn direct_destination(
    workspace_id: &str,
    channel: &str,
    thread_ts: &str,
    message_ts: &str,
) -> (String, serde_json::Value) {
    let thread = SlackThread {
        team: workspace_id.to_string(),
        channel: channel.to_string(),
        thread_ts: thread_ts.to_string(),
        ts: message_ts.to_string(),
    };
    (
        thread.subject_key(),
        serde_json::json!({
            "channel": thread.channel,
            "ts": thread.ts,
            "thread_ts": thread.thread_ts,
        }),
    )
}

/// The subject kind every Slack Binding carries.
pub(super) const fn subject_kind() -> &'static str {
    SUBJECT_KIND
}

/// The Binding a thread-properties bag names.
///
/// Infallible, unlike the trait method below: no Slack destination can fail to
/// be built, because every field of the bag has a fallback. Ingress calls this
/// one so no unreachable failure arm has to exist there.
pub(super) fn thread_destination(
    source_id: &str,
    external_thread_id: &str,
    thread_properties: &serde_json::Value,
    filter_user_id: Option<&str>,
) -> BindingDestination {
    SlackThread::from_thread_properties(thread_properties, source_id, external_thread_id)
        .binding_destination(filter_user_id)
}

impl AdapterDestinationBuilder for SlackAdapter {
    fn binding_destination_from_existing_thread(
        &self,
        source_id: &str,
        external_thread_id: &str,
        thread_properties: &serde_json::Value,
        filter_user_id: Option<&str>,
    ) -> Result<BindingDestination, AdapterError> {
        Ok(thread_destination(
            source_id,
            external_thread_id,
            thread_properties,
            filter_user_id,
        ))
    }
}

#[cfg(test)]
mod tests {
    use moatless_loops::adapters::AdapterDestinationBuilder;
    use moatless_types::adapter::BindingContext;

    use crate::adapter::SlackAdapter;
    use super::{direct_destination, parse_destination, reaction_target};

    fn adapter() -> SlackAdapter {
        SlackAdapter::new("token", "secret", None)
    }

    #[test]
    fn binding_destination_from_existing_thread_uses_workspace_channel_and_thread() {
        let destination = adapter()
            .binding_destination_from_existing_thread(
                "C123",
                "1700000000.000100",
                &serde_json::json!({
                    "channel": "C123",
                    "ts": "1700000000.000100",
                    "thread_ts": "1700000000.000100",
                    "team_id": "T999",
                }),
                Some("user-1"),
            )
            .unwrap();

        assert_eq!(destination.subject_kind, "slack_thread");
        assert_eq!(destination.subject_key, "T999:C123:1700000000.000100");
        assert_eq!(destination.destination_config["channel"], "C123");
        assert_eq!(
            destination.destination_config["thread_ts"],
            "1700000000.000100"
        );
        assert_eq!(destination.destination_config["filter_user_id"], "user-1");
        assert_eq!(destination.last_external_event_id, None);
    }

    /// The bag the tracked-thread lookup builds carries no `ts` while the
    /// admission path passes the real one. Both must keep producing the same
    /// subject key, which is what let the two literals diverge in the first
    /// place.
    #[test]
    fn a_bag_without_a_ts_keys_the_same_subject() {
        let with_ts = adapter()
            .binding_destination_from_existing_thread(
                "C1",
                "1700000000.000100",
                &serde_json::json!({
                    "channel": "C1",
                    "ts": "1700000000.000200",
                    "thread_ts": "1700000000.000100",
                    "team_id": "T1",
                }),
                None,
            )
            .unwrap();
        let without_ts = adapter()
            .binding_destination_from_existing_thread(
                "C1",
                "1700000000.000100",
                &serde_json::json!({
                    "channel": "C1",
                    "thread_ts": "1700000000.000100",
                    "team_id": "T1",
                }),
                None,
            )
            .unwrap();

        assert_eq!(with_ts.subject_key, without_ts.subject_key);
        assert_eq!(with_ts.destination_config["ts"], "1700000000.000200");
        assert_eq!(
            without_ts.destination_config["ts"], "1700000000.000100",
            "an absent ts falls back to the thread anchor"
        );
    }

    /// Every field of the bag has a fallback, and each one is exercised here:
    /// an empty bag is keyed entirely from the event's own coordinates.
    #[test]
    fn an_empty_bag_falls_back_to_the_source_and_thread_ids() {
        let destination = adapter()
            .binding_destination_from_existing_thread(
                "C1",
                "1700000000.000100",
                &serde_json::json!({}),
                None,
            )
            .unwrap();

        assert_eq!(destination.subject_key, "default:C1:1700000000.000100");
        assert_eq!(
            destination.destination_config,
            serde_json::json!({
                "channel": "C1",
                "thread_ts": "1700000000.000100",
                "ts": "1700000000.000100",
            })
        );
    }

    #[test]
    fn an_empty_team_id_is_the_default_workspace() {
        let destination = adapter()
            .binding_destination_from_existing_thread(
                "C1",
                "1700000000.000100",
                &serde_json::json!({"team_id": ""}),
                None,
            )
            .unwrap();

        assert_eq!(destination.subject_key, "default:C1:1700000000.000100");
    }

    #[test]
    fn every_destination_shape_parses() {
        assert_eq!(
            parse_destination("channel:C123").unwrap(),
            ("C123".to_string(), None)
        );
        assert_eq!(
            parse_destination("dm:D123").unwrap(),
            ("D123".to_string(), None)
        );
        assert_eq!(
            parse_destination("thread:C123/1700000000.000100").unwrap(),
            ("C123".to_string(), Some("1700000000.000100".to_string()))
        );
        assert_eq!(
            parse_destination("  C123  ").unwrap(),
            ("C123".to_string(), None),
            "a bare channel id stays accepted for backward compatibility"
        );
    }

    #[test]
    fn a_malformed_destination_names_what_is_wrong() {
        for (destination, expected) in [
            ("", "Invalid request: Slack destination must not be empty"),
            (
                "thread:C123",
                "Invalid request: Slack thread destination must be `thread:<channel>/<thread_ts>`",
            ),
            (
                "thread:/1700000000.000100",
                "Invalid request: Slack thread destination requires both channel and thread_ts",
            ),
            (
                "thread:C123/",
                "Invalid request: Slack thread destination requires both channel and thread_ts",
            ),
            (
                "channel:",
                "Invalid request: Slack channel destination must not be empty",
            ),
            (
                "dm:",
                "Invalid request: Slack DM destination must not be empty",
            ),
        ] {
            let error = parse_destination(destination).unwrap_err();
            assert_eq!(error.to_string(), expected, "for `{destination}`");
        }
    }

    fn binding_context(destination_config: serde_json::Value) -> BindingContext {
        let now = chrono::Utc::now();
        BindingContext {
            binding_id: "tb_1".into(),
            loop_id: None,
            task_id: "task-1".into(),
            subject_kind: "slack_thread".into(),
            subject_key: "T1:C1:1700000000.000100".into(),
            destination_config,
            metadata: serde_json::json!({}),
            last_external_event_id: None,
            last_message_at: now,
            created_at: now,
        }
    }

    #[test]
    fn a_reaction_target_pairs_the_binding_channel_with_the_message_ts() {
        let binding = binding_context(serde_json::json!({"channel": "C1"}));

        assert_eq!(
            reaction_target(&binding, "slack:ts:1700000000.000100").unwrap(),
            ("C1".to_string(), "1700000000.000100")
        );
        assert_eq!(
            reaction_target(&binding, "1700000000.000100").unwrap(),
            ("C1".to_string(), "1700000000.000100"),
            "a legacy bare ts still resolves"
        );
    }

    #[test]
    fn a_reaction_target_reports_each_missing_coordinate() {
        let binding = binding_context(serde_json::json!({"channel": "C1"}));
        assert_eq!(
            reaction_target(&binding, "github:ic:42")
                .unwrap_err()
                .to_string(),
            "Invalid request: Slack reaction target `github:ic:42` is not a valid slack:ts:<ts> id"
        );

        let no_channel = binding_context(serde_json::json!({}));
        assert_eq!(
            reaction_target(&no_channel, "slack:ts:1700000000.000100")
                .unwrap_err()
                .to_string(),
            "Invalid request: Slack binding destination_config has no `channel` field"
        );
    }

    /// A direct post binds under the workspace `auth.test` reported, and a
    /// top-level post's own `ts` is its thread anchor.
    #[test]
    fn a_direct_post_binds_under_the_workspace_channel_and_thread() {
        let (key, config) =
            direct_destination("T1", "C1", "1700000000.000100", "1700000000.000100");
        assert_eq!(key, "T1:C1:1700000000.000100");
        assert_eq!(
            config,
            serde_json::json!({
                "channel": "C1",
                "ts": "1700000000.000100",
                "thread_ts": "1700000000.000100",
            })
        );

        let (reply_key, reply_config) =
            direct_destination("T1", "C1", "1700000000.000100", "1700000000.000900");
        assert_eq!(
            reply_key, "T1:C1:1700000000.000100",
            "a thread reply keys under the parent, not under itself"
        );
        assert_eq!(reply_config["ts"], "1700000000.000900");
    }
}
