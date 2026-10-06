//! Re-running a turn its provider would not serve, on the other agent.

use std::collections::HashMap;

use uuid::Uuid;

use crate::inbox::models::{DeliveryOptions, InboxItem, InboxOrigin, NewInboxItem};

use super::enricher::build_message_header;

/// A failed turn's input, folded into the one item that re-runs it.
#[derive(Debug, Clone)]
pub struct TurnReplay {
    pub(crate) item: NewInboxItem,
    /// The failed turn was itself a fallback, so it must not fall back again.
    pub was_fallback: bool,
}

/// The item that re-runs `items`, the ones `turn_number` delivered; `None` when
/// it delivered none.
///
/// Its id derives from the turn, so a redelivered `agent.error` queues nothing
/// twice. Model and effort are dropped: they name the other agent's catalog.
pub fn fallback_replay(task_id: &str, turn_number: i32, items: &[InboxItem]) -> Option<TurnReplay> {
    let first = items.first()?;
    let last = items.last()?;
    let no_logins = HashMap::new();
    let text = items
        .iter()
        .map(|item| {
            let header = match item.origin {
                InboxOrigin::Adapter { .. } => build_message_header(&item.as_message(), &no_logins),
                _ => String::new(),
            };
            format!("{header}{}", item.text)
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut skills: Vec<String> = Vec::new();
    for skill in items.iter().flat_map(|item| &item.options.skills) {
        if !skills.contains(skill) {
            skills.push(skill.clone());
        }
    }
    let key = format!("usage-limit-fallback\u{1f}{task_id}\u{1f}{turn_number}");
    Some(TurnReplay {
        item: NewInboxItem {
            id: Uuid::new_v5(&Uuid::NAMESPACE_OID, key.as_bytes()),
            task_id: task_id.to_string(),
            text,
            file_ids: items.iter().flat_map(|i| i.file_ids.clone()).collect(),
            options: DeliveryOptions {
                model: None,
                agent_mode: last.options.agent_mode.clone(),
                effort: None,
                skills,
                resume: last.options.resume,
                start_fresh: last.options.start_fresh,
                queue: false,
                contexts: items
                    .iter()
                    .flat_map(|i| i.options.contexts.clone())
                    .collect(),
                tool_response: None,
                agent_fallback: true,
            },
            answers_tool_use_id: None,
            // An Adapter channel is keyed on its external message id, which the
            // original row already holds.
            origin: match &first.origin {
                origin @ (InboxOrigin::User { .. }
                | InboxOrigin::Task { .. }
                | InboxOrigin::Sandbox) => origin.clone(),
                InboxOrigin::Adapter { .. } | InboxOrigin::System | InboxOrigin::Unknown => {
                    InboxOrigin::System
                }
            },
        },
        was_fallback: items.iter().any(|item| item.options.agent_fallback),
    })
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::inbox::models::{AdapterChannel, ExternalSender, InboxStatus};

    fn item(text: &str, origin: InboxOrigin, options: DeliveryOptions) -> InboxItem {
        InboxItem {
            id: Uuid::new_v4(),
            task_id: "t1".into(),
            created_at: Utc::now(),
            text: text.into(),
            file_ids: vec![],
            options,
            answers_tool_use_id: None,
            origin,
            status: InboxStatus::Delivered,
            delivered_at: Some(Utc::now()),
            run_id: Some("run".into()),
            turn_number: Some(3),
            session_id: Some("sess".into()),
        }
    }

    #[test]
    fn a_turn_with_nothing_delivered_has_no_replay() {
        assert!(fallback_replay("t1", 3, &[]).is_none());
    }

    #[test]
    fn the_replay_keeps_the_prompt_and_drops_the_other_agents_model() {
        let scheduled = item(
            "/review-frontend",
            InboxOrigin::System,
            DeliveryOptions {
                model: Some("gpt-5.5".into()),
                effort: Some("high".into()),
                skills: vec!["review".into()],
                ..DeliveryOptions::default()
            },
        );
        let replay = fallback_replay("t1", 3, &[scheduled]).expect("a replay");
        assert!(!replay.was_fallback);
        assert_eq!(replay.item.text, "/review-frontend");
        assert_eq!(replay.item.options.model, None);
        assert_eq!(replay.item.options.effort, None);
        assert_eq!(replay.item.options.skills, vec!["review".to_string()]);
        assert!(replay.item.options.agent_fallback);
        assert_eq!(replay.item.origin, InboxOrigin::System);
        assert_eq!(
            replay.item.id,
            fallback_replay(
                "t1",
                3,
                &[item("x", InboxOrigin::System, DeliveryOptions::default())]
            )
            .unwrap()
            .item
            .id,
            "the id is the turn's, so a redelivered error queues nothing twice"
        );
    }

    #[test]
    fn an_adapter_item_keeps_its_reply_header() {
        let comment = item(
            "Please re-check the null handling.",
            InboxOrigin::Adapter {
                channel: AdapterChannel {
                    adapter: "github_pr".into(),
                    external_message_id: "c-1".into(),
                    subject_key: Some("org/repo#7".into()),
                    source_name: None,
                },
                sender: ExternalSender {
                    id: None,
                    author: Some("octocat".into()),
                },
                sent_by: None,
            },
            DeliveryOptions::default(),
        );
        let id = comment.id;
        let replay = fallback_replay("t1", 3, &[comment]).expect("a replay");
        assert!(
            replay.item.text.starts_with(&format!(
                "<message_id>{id}</message_id>\n<source>github_pr</source>"
            )),
            "{}",
            replay.item.text
        );
        assert!(
            replay
                .item
                .text
                .ends_with("Please re-check the null handling.")
        );
        assert_eq!(replay.item.origin, InboxOrigin::System);
    }

    #[test]
    fn a_turn_a_fallback_ran_is_marked() {
        let fallback = item(
            "/review-frontend",
            InboxOrigin::System,
            DeliveryOptions {
                agent_fallback: true,
                ..DeliveryOptions::default()
            },
        );
        assert!(fallback_replay("t1", 4, &[fallback]).unwrap().was_fallback);
    }
}
