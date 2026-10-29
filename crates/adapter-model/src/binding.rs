//! The Binding domain the Adapter kernel publishes: the read seam, the origin
//! taxonomy, and the vocabulary a provider builds from an inbound event.
//!
//! An Adapter that only *asks about* Bindings reads [`BindingProbe`],
//! implemented by the Binding store in `moatless-loops` beside its table; one
//! that *drives* the Binding lifecycle names that store directly.
//!
//! Origin constants are the declared `task_bindings.origin` values adapter
//! routing uses — the column is an open string, and other writers use values
//! outside this set.

use async_trait::async_trait;

use chrono::{DateTime, Utc};
use moatless_types::adapter::TaskBinding;
use moatless_types::store::StoreError;

/// Whether a subject already has a Binding. Answers a yes/no; the rows behind
/// it stay in the crate that owns the table.
#[async_trait]
pub trait BindingProbe: Send + Sync {
    /// Whether any Binding tracks this subject.
    async fn is_bound(
        &self,
        adapter_kind: &str,
        subject_kind: &str,
        subject_key: &str,
    ) -> Result<bool, StoreError>;

    /// Whether an active Binding tracks this subject. A deactivated one, which
    /// [`Self::is_bound`] counts, does not.
    async fn is_actively_bound(
        &self,
        adapter_kind: &str,
        subject_kind: &str,
        subject_key: &str,
    ) -> Result<bool, StoreError>;
}

/// The `task_bindings.origin` role taxonomy: the direction the task and
/// external subject met.
///
/// This is the responder direction — the router writes it from an inbound
/// event; every other origin is a producer where the task existed first. Values
/// are fixed: existing rows and the API depend on them.
pub const ADAPTER_EVENT_BINDING_ORIGIN: &str = "ingress";
/// Producer: the task replied into a subject, or was connected to one.
pub const CONNECTED_THREAD_BINDING_ORIGIN: &str = "connected_thread";
/// Producer: a CLI-driven endpoint filed the subject on the task's behalf.
pub const TASK_OWNED_BINDING_ORIGIN: &str = "task_owned";
/// Producer: the Task's own Task Room, opened for it. A reply on the room's
/// Adapter goes here unless it names a subject other than an `ingress` one.
pub const ROOM_BINDING_ORIGIN: &str = "room";

/// The role a Binding's task holds on its subject, in `metadata.bind_reason`.
///
/// A [`Self::Commenter`] hears only what its adapter marks as answering its
/// writing; every other role hears every event. Weakest first, persisted:
/// `PgTaskBindingDao::upsert` keeps the strongest a row has held, in this order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BindReason {
    /// The task wrote on the subject without working it, as a comment does.
    Commenter,
    /// The task changed the subject's content, as a push to a PR branch does.
    Contributor,
    /// The task, or a person for it, asked to follow the subject by name.
    Follower,
    /// The task created the subject.
    Creator,
}

/// The `metadata` key holding a Binding's [`BindReason`].
pub const BIND_REASON_KEY: &str = "bind_reason";
/// The `metadata` key holding the reply keys of what a Binding's task wrote.
///
/// A reply key is an adapter-defined string an inbound event lists in
/// `InboundMessage::replies_to` when it answers that writing.
pub const REPLY_KEYS_KEY: &str = "reply_keys";
/// The `metadata` key that, when `true`, closes a Binding's task when its
/// subject closes, as a trigger's `close_task_on_close` does for the tasks the
/// trigger created.
pub const CLOSE_ON_SUBJECT_CLOSE_KEY: &str = "close_on_subject_close";

impl BindReason {
    /// The persisted spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Commenter => "commenter",
            Self::Contributor => "contributor",
            Self::Follower => "follower",
            Self::Creator => "creator",
        }
    }

    /// The role `binding` records. `None` for a row written before roles
    /// were recorded and for a writer that records none: both keep hearing
    /// every event, so `None` must never be read as [`Self::Commenter`].
    pub fn of(binding: &TaskBinding) -> Option<Self> {
        match binding.metadata.get(BIND_REASON_KEY)?.as_str()? {
            "commenter" => Some(Self::Commenter),
            "contributor" => Some(Self::Contributor),
            "follower" => Some(Self::Follower),
            "creator" => Some(Self::Creator),
            _ => None,
        }
    }
}

/// Whether `binding`'s task hears only replies to what it wrote.
pub fn hears_only_replies(binding: &TaskBinding) -> bool {
    BindReason::of(binding) == Some(BindReason::Commenter)
}

/// Whether any of `replies_to` names something `binding`'s task wrote.
pub fn holds_reply_key(binding: &TaskBinding, replies_to: &[String]) -> bool {
    binding
        .metadata
        .get(REPLY_KEYS_KEY)
        .and_then(serde_json::Value::as_array)
        .is_some_and(|keys| {
            keys.iter()
                .filter_map(serde_json::Value::as_str)
                .any(|key| replies_to.iter().any(|reply| reply == key))
        })
}

/// Whether `binding`'s task asked to be closed when its subject closes.
pub fn closes_on_subject_close(binding: &TaskBinding) -> bool {
    binding
        .metadata
        .get(CLOSE_ON_SUBJECT_CLOSE_KEY)
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

/// What a bind records beyond the subject: why, what the task wrote there, and
/// whether the subject closing closes the task.
#[derive(Debug, Clone, Default)]
pub struct BindScope {
    /// `None` writes no reason, leaving the row hearing every event.
    pub reason: Option<BindReason>,
    /// Added to the keys the row already holds, never replacing them.
    pub reply_keys: Vec<String>,
    /// `false` writes nothing, so a later bind never clears an earlier opt-in.
    pub close_on_subject_close: bool,
}

impl BindScope {
    /// A bind for `reason` that records no writing.
    pub fn because(reason: BindReason) -> Self {
        Self {
            reason: Some(reason),
            reply_keys: Vec::new(),
            close_on_subject_close: false,
        }
    }

    /// Write this scope into a new row's `metadata`.
    pub fn write_into(&self, metadata: &mut serde_json::Map<String, serde_json::Value>) {
        if let Some(reason) = self.reason {
            metadata.insert(BIND_REASON_KEY.into(), reason.as_str().into());
        }
        if !self.reply_keys.is_empty() {
            metadata.insert(REPLY_KEYS_KEY.into(), self.reply_keys.clone().into());
        }
        if self.close_on_subject_close {
            metadata.insert(CLOSE_ON_SUBJECT_CLOSE_KEY.into(), true.into());
        }
    }
}

/// Build a provider binding context from a task binding. Never persist this
/// row.
pub fn binding_context(binding: &TaskBinding) -> BindingContext {
    let mut destination_config = binding.destination_config.clone();
    if let Some(obj) = destination_config.as_object_mut() {
        if let Some(ext_id) = binding.last_external_event_id.as_ref() {
            obj.insert("last_external_event_id".into(), serde_json::json!(ext_id));
        }
        if let Some(filter_uid) = binding.metadata.get("filter_user_id").cloned() {
            obj.insert("filter_user_id".into(), filter_uid);
        }
    }

    BindingContext {
        binding_id: binding.id.clone(),
        loop_id: binding.loop_id.clone(),
        task_id: binding.task_id.clone(),
        subject_kind: binding.subject_kind.clone(),
        subject_key: binding.subject_key.clone(),
        destination_config,
        metadata: binding.metadata.clone(),
        last_external_event_id: binding.last_external_event_id.clone(),
        last_message_at: binding.last_event_at,
        created_at: binding.created_at,
    }
}

/// Adapter-facing subject context.
///
/// Synthesized from a task binding before invoking Adapter-specific outbound
/// and context-fetching code.
#[derive(Debug, Clone)]
pub struct BindingContext {
    pub binding_id: String,
    pub loop_id: Option<String>,
    pub task_id: String,
    pub subject_kind: String,
    pub subject_key: String,
    pub destination_config: serde_json::Value,
    pub metadata: serde_json::Value,
    pub last_external_event_id: Option<String>,
    pub last_message_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

/// Provider-owned destination data for a task binding.
#[derive(Debug, Clone)]
pub struct BindingDestination {
    /// Binding `subject_kind` (for example, a provider-defined thread kind).
    pub subject_kind: &'static str,
    /// Stable provider-defined lookup key for the bound subject.
    pub subject_key: String,
    /// Provider-defined JSON needed to send future replies.
    pub destination_config: serde_json::Value,
    /// Optional cursor seeded when the binding is created from an outbound
    /// send.
    pub last_external_event_id: Option<String>,
}

impl BindingDestination {
    pub fn new(
        subject_kind: &'static str,
        subject_key: String,
        destination_config: serde_json::Value,
    ) -> Self {
        Self {
            subject_kind,
            subject_key,
            destination_config,
            last_external_event_id: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BIND_REASON_KEY, BindReason, BindScope, CLOSE_ON_SUBJECT_CLOSE_KEY,
        closes_on_subject_close, hears_only_replies, holds_reply_key,
    };
    use chrono::Utc;
    use moatless_types::adapter::{AdapterKind, TaskBinding};

    fn with_metadata(metadata: serde_json::Value) -> TaskBinding {
        let now = Utc::now();
        TaskBinding {
            id: "ts_1".into(),
            task_id: "t_1".into(),
            adapter_kind: AdapterKind::Github,
            subject_kind: "pull_request".into(),
            subject_key: "acme/widgets#1".into(),
            connection_id: "pc_1".into(),
            loop_id: None,
            destination_config: serde_json::json!({}),
            metadata,
            last_external_event_id: None,
            origin: "connected_thread".into(),
            created_at: now,
            last_event_at: now,
            external_status: None,
            external_state_kind: None,
            external_status_raw: None,
            external_review_decision: None,
            external_mergeable_state: None,
            external_status_updated_at: None,
            active: true,
            deactivated_at: None,
            watching: true,
        }
    }

    fn scoped(scope: &BindScope) -> TaskBinding {
        let mut metadata = serde_json::Map::new();
        scope.write_into(&mut metadata);
        with_metadata(metadata.into())
    }

    /// Weakest first, the order `PgTaskBindingDao::upsert` ranks in SQL.
    #[test]
    fn every_reason_reads_back_in_rank_order() {
        let ranked = [
            BindReason::Commenter,
            BindReason::Contributor,
            BindReason::Follower,
            BindReason::Creator,
        ];
        assert!(ranked.windows(2).all(|pair| pair[0] < pair[1]));
        for reason in ranked {
            assert_eq!(
                BindReason::of(&scoped(&BindScope::because(reason))),
                Some(reason)
            );
        }
    }

    /// A row with no reason, or one this build does not know, hears everything.
    #[test]
    fn only_a_recorded_comment_narrows_a_binding() {
        assert!(hears_only_replies(&scoped(&BindScope::because(
            BindReason::Commenter
        ))));
        assert!(!hears_only_replies(&with_metadata(serde_json::json!({}))));
        assert!(!hears_only_replies(&with_metadata(
            serde_json::json!({ BIND_REASON_KEY: "later" })
        )));
    }

    #[test]
    fn a_reply_key_matches_only_what_the_task_wrote() {
        let row = scoped(&BindScope {
            reason: Some(BindReason::Commenter),
            reply_keys: vec!["k:1".into(), "k:2".into()],
            close_on_subject_close: false,
        });
        assert!(holds_reply_key(&row, &["k:9".into(), "k:2".into()]));
        assert!(!holds_reply_key(&row, &["k:9".into()]));
        assert!(!holds_reply_key(&row, &[]));
    }

    #[test]
    fn only_an_opted_in_bind_closes_with_its_subject() {
        let opted_in = scoped(&BindScope {
            close_on_subject_close: true,
            ..BindScope::because(BindReason::Creator)
        });
        assert!(closes_on_subject_close(&opted_in));
        assert!(!closes_on_subject_close(&scoped(&BindScope::because(
            BindReason::Creator
        ))));
        assert!(!closes_on_subject_close(&with_metadata(
            serde_json::json!({ CLOSE_ON_SUBJECT_CLOSE_KEY: "yes" })
        )));
    }
}
