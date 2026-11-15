//! Which bindings receive an inbound event — decided only here.
//!
//! [`resolve_recipients`] does no I/O: the active bindings, the event's
//! author task, and the parent author an [`Audience::ParentAuthor`] names
//! are all resolved by the router first, making the matrix cheap to test.
//!
//! Identity plays no part: a task is excluded only when it can be *named*
//! as the author, never by matching a sender identity — which would mute
//! anyone posting under the same identity a provider uses.

use moatless_adapter_model::binding::ADAPTER_EVENT_BINDING_ORIGIN;
use moatless_adapter_model::binding::{hears_only_replies, holds_reply_key};
use moatless_types::adapter::Audience;
use moatless_types::adapter::TaskBinding;

/// What the event says about who wrote it, resolved before the resolver runs.
///
/// Both fields are task ids recovered from recorded messages; an event carrying
/// neither arrives with both `None` and is treated as authored by nobody, a
/// case that must still reach the producing task.
#[derive(Debug, Clone, Copy)]
pub struct EventProvenance<'a> {
    /// The task that made the event itself, when it can be named.
    pub made_by: Option<&'a str>,
    /// The task that authored the parent event named by
    /// [`Audience::ParentAuthor`]. `None` for every other audience, and for a
    /// parent that resolves to no task.
    pub parent_author_task_id: Option<&'a str>,
    /// The event's [`InboundMessage::replies_to`](moatless_types::adapter::InboundMessage::replies_to).
    pub replies_to: &'a [String],
}

/// The bindings that should receive an event with this `audience`, in
/// `bindings` order with no duplicates.
///
/// Order matters: the audience **names** bindings first, then the event's
/// author is **excluded** — excluding first would conflate "named nobody" (fall
/// back) with "named exactly the author" (deliver to nobody).
pub fn resolve_recipients<'b>(
    audience: &Audience,
    bindings: &'b [TaskBinding],
    provenance: EventProvenance<'_>,
) -> Vec<&'b TaskBinding> {
    let named: Vec<&TaskBinding> = match audience {
        Audience::Everyone => bindings.iter().collect(),
        // The fallback in both arms below reads "the audience named nobody",
        // never "everyone it named wrote the event". A signal about a subject
        // nobody produced still reaches the tasks working it; a reply a task
        // wrote inside its own thread reaches nobody, because the one task it
        // was for is the one that wrote it.
        Audience::SubjectProducer => or_everyone(named_by(bindings, is_producer), bindings),
        Audience::ParentAuthor { .. } => or_everyone(
            named_by(bindings, |binding| {
                is_producer(binding) || answers(binding, provenance)
            }),
            bindings,
        ),
    };

    // A task never receives back what it is named as the author of, and a
    // binding with its watch off hears nothing.
    named
        .into_iter()
        .filter(|binding| binding.watching)
        .filter(|binding| !hears_only_replies(binding) || answers(binding, provenance))
        .filter(|binding| provenance.made_by != Some(binding.task_id.as_str()))
        .collect()
}

/// Whether the event replies to something `binding`'s task wrote: the parent
/// it names, or a reply key the binding recorded. The only way a
/// [`hears_only_replies`] binding, which no fallback names, receives an event.
fn answers(binding: &TaskBinding, provenance: EventProvenance<'_>) -> bool {
    provenance.parent_author_task_id == Some(binding.task_id.as_str())
        || holds_reply_key(binding, provenance.replies_to)
}

fn named_by(
    bindings: &[TaskBinding],
    predicate: impl Fn(&TaskBinding) -> bool,
) -> Vec<&TaskBinding> {
    bindings
        .iter()
        .filter(|binding| predicate(binding))
        .collect()
}

/// An audience that named nobody falls back to the whole subject: no task being
/// nameable is not a reason to deliver to no task.
fn or_everyone<'b>(
    named: Vec<&'b TaskBinding>,
    bindings: &'b [TaskBinding],
) -> Vec<&'b TaskBinding> {
    if named.is_empty() {
        bindings.iter().collect()
    } else {
        named
    }
}

/// A binding formed task-to-subject, the task reaching out first, unless it
/// only commented: a commenter counted as producer takes build results from
/// the review task. Negates the one responder origin, deliberately: `origin`
/// is an open column, and a list of producers would read an undeclared origin
/// as a responder.
fn is_producer(binding: &TaskBinding) -> bool {
    binding.origin != ADAPTER_EVENT_BINDING_ORIGIN && !hears_only_replies(binding)
}

#[cfg(test)]
mod tests {
    use super::{EventProvenance, resolve_recipients};
    use chrono::Utc;
    use moatless_adapter_model::binding::ADAPTER_EVENT_BINDING_ORIGIN;
    use moatless_adapter_model::binding::CONNECTED_THREAD_BINDING_ORIGIN;
    use moatless_adapter_model::binding::TASK_OWNED_BINDING_ORIGIN;
    use moatless_adapter_model::binding::{
        BIND_REASON_KEY, BindReason, REPLY_KEYS_KEY, hears_only_replies, holds_reply_key,
    };
    use moatless_types::adapter::Audience;
    use moatless_types::adapter::TaskBinding;

    /// A binding is identified in these tests by its task id; every other
    /// column is scenery. `origin` is the only field the resolver reads besides
    /// `task_id`, and it carries the task's role on the subject.
    fn binding(task_id: &str, origin: &str) -> TaskBinding {
        let now = Utc::now();
        TaskBinding {
            id: format!("ts_{task_id}"),
            task_id: task_id.to_string(),
            adapter_kind: moatless_types::adapter::AdapterKind::Github,
            subject_kind: "subject".to_string(),
            subject_key: "acme/widgets#1".to_string(),
            connection_id: "pc_1".to_string(),
            loop_id: None,
            destination_config: serde_json::json!({}),
            metadata: serde_json::json!({}),
            last_external_event_id: None,
            origin: origin.to_string(),
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

    /// The task that created the subject — opened the PR, sent to the issue.
    fn producer(task_id: &str) -> TaskBinding {
        binding(task_id, CONNECTED_THREAD_BINDING_ORIGIN)
    }

    /// A task a loop spawned from the subject — a reviewer.
    fn responder(task_id: &str) -> TaskBinding {
        binding(task_id, ADAPTER_EVENT_BINDING_ORIGIN)
    }

    /// A producer bound by a CLI-driven endpoint rather than by a reply — the
    /// origin the Linear issue-creating route writes. A third origin value, and
    /// still the producer direction.
    fn task_owned_producer(task_id: &str) -> TaskBinding {
        binding(task_id, TASK_OWNED_BINDING_ORIGIN)
    }

    /// A task that only commented, holding the reply keys of what it wrote.
    fn commenter(task_id: &str, reply_keys: &[&str]) -> TaskBinding {
        let mut row = binding(task_id, CONNECTED_THREAD_BINDING_ORIGIN);
        row.metadata = serde_json::json!({
            BIND_REASON_KEY: BindReason::Commenter.as_str(),
            REPLY_KEYS_KEY: reply_keys,
        });
        row
    }

    /// A task that created the subject, recorded as such.
    fn creator(task_id: &str) -> TaskBinding {
        let mut row = binding(task_id, CONNECTED_THREAD_BINDING_ORIGIN);
        row.metadata = serde_json::json!({ BIND_REASON_KEY: BindReason::Creator.as_str() });
        row
    }

    fn keys(keys: &[&str]) -> Vec<String> {
        keys.iter().map(|key| (*key).to_string()).collect()
    }

    fn parent_reply(parent: &str) -> Audience {
        Audience::ParentAuthor {
            parent_external_event_id: Some(parent.to_string()),
        }
    }

    fn recipients(
        audience: &Audience,
        bindings: &[TaskBinding],
        provenance: EventProvenance<'_>,
    ) -> Vec<String> {
        resolve_recipients(audience, bindings, provenance)
            .into_iter()
            .map(|binding| binding.task_id.clone())
            .collect()
    }

    /// One row of the matrix: an audience, the roles present on the subject,
    /// and the provenance the event carries.
    struct Row {
        /// What the row demonstrates, quoted back on failure.
        case: &'static str,
        audience: Audience,
        bindings: Vec<TaskBinding>,
        made_by: Option<&'static str>,
        parent_author_task_id: Option<&'static str>,
        replies_to: Vec<String>,
        expected: Vec<&'static str>,
    }

    /// Audience × roles present × provenance carried, with no database.
    ///
    /// The rows are the whole of the routing policy: role eligibility, the
    /// fallback when the preferred role is absent, thread scoping, and the
    /// provenance exclusion.
    fn table() -> Vec<Row> {
        vec![
            // ---- Everyone ----------------------------------------------
            Row {
                case: "everyone, both roles bound, no provenance — both receive",
                audience: Audience::Everyone,
                bindings: vec![producer("t_prod"), responder("t_resp")],
                made_by: None,
                parent_author_task_id: None,
                replies_to: vec![],
                expected: vec!["t_prod", "t_resp"],
            },
            Row {
                case: "everyone, the producer's watch is off — only the responder receives",
                audience: Audience::Everyone,
                bindings: vec![
                    TaskBinding {
                        watching: false,
                        ..producer("t_prod")
                    },
                    responder("t_resp"),
                ],
                made_by: None,
                parent_author_task_id: None,
                replies_to: vec![],
                expected: vec!["t_resp"],
            },
            Row {
                case: "everyone, no bindings at all — nobody receives",
                audience: Audience::Everyone,
                bindings: vec![],
                made_by: None,
                parent_author_task_id: None,
                replies_to: vec![],
                expected: vec![],
            },
            Row {
                case: "everyone, the producer authored it — only the responder receives",
                audience: Audience::Everyone,
                bindings: vec![producer("t_prod"), responder("t_resp")],
                made_by: Some("t_prod"),
                parent_author_task_id: None,
                replies_to: vec![],
                expected: vec!["t_resp"],
            },
            Row {
                case: "everyone, the responder authored it — only the producer receives",
                audience: Audience::Everyone,
                bindings: vec![producer("t_prod"), responder("t_resp")],
                made_by: Some("t_resp"),
                parent_author_task_id: None,
                replies_to: vec![],
                expected: vec!["t_prod"],
            },
            Row {
                case: "everyone, authored by a task bound elsewhere — everyone still receives",
                audience: Audience::Everyone,
                bindings: vec![producer("t_prod"), responder("t_resp")],
                made_by: Some("t_stranger"),
                parent_author_task_id: None,
                replies_to: vec![],
                expected: vec!["t_prod", "t_resp"],
            },
            Row {
                case: "everyone, only binding authored it — nobody receives",
                audience: Audience::Everyone,
                bindings: vec![producer("t_prod")],
                made_by: Some("t_prod"),
                parent_author_task_id: None,
                replies_to: vec![],
                expected: vec![],
            },
            // ---- The subject's producer --------------------------------
            Row {
                // The row that failed in production: a build result or merge
                // conflict carrying no provenance whatsoever, on a PR a task
                // opened, resolves to that task rather than to nothing.
                case: "producer, both roles bound, no provenance — the producer receives",
                audience: Audience::SubjectProducer,
                bindings: vec![producer("t_prod"), responder("t_resp")],
                made_by: None,
                parent_author_task_id: None,
                replies_to: vec![],
                expected: vec!["t_prod"],
            },
            Row {
                case: "producer, only a responder bound — it receives as the fallback",
                audience: Audience::SubjectProducer,
                bindings: vec![responder("t_resp"), responder("t_resp2")],
                made_by: None,
                parent_author_task_id: None,
                replies_to: vec![],
                expected: vec!["t_resp", "t_resp2"],
            },
            Row {
                // Unreachable in practice, kept for the shape: this audience
                // carries build/merge signals no task authors. If one ever
                // were, the producer is who it was for — the fallback asks
                // whether the audience named anyone, not whether the named
                // task survived.
                case: "producer, the producer authored it — nobody receives, no fallback",
                audience: Audience::SubjectProducer,
                bindings: vec![producer("t_prod"), responder("t_resp")],
                made_by: Some("t_prod"),
                parent_author_task_id: None,
                replies_to: vec![],
                expected: vec![],
            },
            Row {
                case: "producer, two producers bound — both receive",
                audience: Audience::SubjectProducer,
                bindings: vec![producer("t_prod"), producer("t_prod2"), responder("t_resp")],
                made_by: None,
                parent_author_task_id: None,
                replies_to: vec![],
                expected: vec!["t_prod", "t_prod2"],
            },
            Row {
                case: "producer, the producer filed the subject from the CLI — it receives",
                audience: Audience::SubjectProducer,
                bindings: vec![task_owned_producer("t_prod"), responder("t_resp")],
                made_by: None,
                parent_author_task_id: None,
                replies_to: vec![],
                expected: vec!["t_prod"],
            },
            Row {
                case: "producer, no bindings at all — nobody receives",
                audience: Audience::SubjectProducer,
                bindings: vec![],
                made_by: None,
                parent_author_task_id: None,
                replies_to: vec![],
                expected: vec![],
            },
            // ---- A named parent's author -------------------------------
            Row {
                case: "parent's author, the parent is a responder — it and the producer receive",
                audience: parent_reply("evt:parent"),
                bindings: vec![
                    producer("t_prod"),
                    responder("t_resp"),
                    responder("t_other"),
                ],
                made_by: None,
                parent_author_task_id: Some("t_resp"),
                replies_to: vec![],
                expected: vec!["t_prod", "t_resp"],
            },
            Row {
                case: "parent's author, the parent is the producer — only the producer receives",
                audience: parent_reply("evt:parent"),
                bindings: vec![producer("t_prod"), responder("t_resp")],
                made_by: None,
                parent_author_task_id: Some("t_prod"),
                replies_to: vec![],
                expected: vec!["t_prod"],
            },
            Row {
                case: "parent's author, the parent resolves to no task — only the producer receives",
                audience: parent_reply("evt:parent"),
                bindings: vec![producer("t_prod"), responder("t_resp")],
                made_by: None,
                parent_author_task_id: None,
                replies_to: vec![],
                expected: vec!["t_prod"],
            },
            Row {
                case: "parent's author, a reply starting its own thread — only the producer receives",
                audience: Audience::ParentAuthor {
                    parent_external_event_id: None,
                },
                bindings: vec![producer("t_prod"), responder("t_resp")],
                made_by: None,
                parent_author_task_id: None,
                replies_to: vec![],
                expected: vec!["t_prod"],
            },
            Row {
                case: "parent's author, the producer filed the subject from the CLI — it receives with the parent's author",
                audience: parent_reply("evt:parent"),
                bindings: vec![
                    task_owned_producer("t_prod"),
                    responder("t_resp"),
                    responder("t_other"),
                ],
                made_by: None,
                parent_author_task_id: Some("t_resp"),
                replies_to: vec![],
                expected: vec!["t_prod", "t_resp"],
            },
            Row {
                case: "parent's author, no producer bound — the parent's author alone, the fallback does not apply",
                audience: parent_reply("evt:parent"),
                bindings: vec![responder("t_resp"), responder("t_other")],
                made_by: None,
                parent_author_task_id: Some("t_resp"),
                replies_to: vec![],
                expected: vec!["t_resp"],
            },
            Row {
                case: "parent's author, the parent's author wrote the reply too — the producer receives",
                audience: parent_reply("evt:parent"),
                bindings: vec![producer("t_prod"), responder("t_resp")],
                made_by: Some("t_resp"),
                parent_author_task_id: Some("t_resp"),
                replies_to: vec![],
                expected: vec!["t_prod"],
            },
            Row {
                // Not the fallback: the audience named the producer, and the
                // producer wrote the reply. We know exactly whose event it was
                // and they already have it, so no other task is woken by a task
                // talking to itself.
                case: "parent's author, the producer wrote the reply in its own thread — nobody receives",
                audience: parent_reply("evt:parent"),
                bindings: vec![producer("t_prod"), responder("t_resp")],
                made_by: Some("t_prod"),
                parent_author_task_id: Some("t_prod"),
                replies_to: vec![],
                expected: vec![],
            },
            Row {
                // The common Linear shape: an issue a loop spawned a task from,
                // so no producer, and two humans threading a conversation the
                // task never wrote into. Every bound task hears it, exactly as
                // an issue-level comment would be heard.
                case: "parent's author, no producer and a parent no task wrote — everyone bound receives as the fallback",
                audience: parent_reply("evt:parent"),
                bindings: vec![responder("t_resp"), responder("t_other")],
                made_by: None,
                parent_author_task_id: None,
                replies_to: vec![],
                expected: vec!["t_resp", "t_other"],
            },
            Row {
                case: "parent's author, no producer and a reply starting its own thread — everyone bound receives as the fallback",
                audience: Audience::ParentAuthor {
                    parent_external_event_id: None,
                },
                bindings: vec![responder("t_resp"), responder("t_other")],
                made_by: None,
                parent_author_task_id: None,
                replies_to: vec![],
                expected: vec!["t_resp", "t_other"],
            },
            // ---- A task that only commented ----------------------------
            Row {
                case: "everyone, a push with a commenter bound — only the review task receives",
                audience: Audience::Everyone,
                bindings: vec![commenter("t_comm", &["k:thread"]), responder("t_resp")],
                made_by: None,
                parent_author_task_id: None,
                replies_to: vec![],
                expected: vec!["t_resp"],
            },
            Row {
                case: "everyone, a push with only a commenter bound — nobody receives",
                audience: Audience::Everyone,
                bindings: vec![commenter("t_comm", &["k:thread"])],
                made_by: None,
                parent_author_task_id: None,
                replies_to: vec![],
                expected: vec![],
            },
            Row {
                case: "everyone, a conversation comment answering the commenter — both receive",
                audience: Audience::Everyone,
                bindings: vec![commenter("t_comm", &["k:comment"]), responder("t_resp")],
                made_by: None,
                parent_author_task_id: None,
                replies_to: keys(&["k:other", "k:comment"]),
                expected: vec!["t_comm", "t_resp"],
            },
            Row {
                // The row that failed in production: a commenter counted as
                // the producer took build results from the review task.
                case: "producer, a build result with a commenter bound — the review task receives",
                audience: Audience::SubjectProducer,
                bindings: vec![commenter("t_comm", &[]), responder("t_resp")],
                made_by: None,
                parent_author_task_id: None,
                replies_to: vec![],
                expected: vec!["t_resp"],
            },
            Row {
                case: "producer, the creator and a commenter bound — only the creator receives",
                audience: Audience::SubjectProducer,
                bindings: vec![
                    commenter("t_comm", &[]),
                    creator("t_prod"),
                    responder("t_resp"),
                ],
                made_by: None,
                parent_author_task_id: None,
                replies_to: vec![],
                expected: vec!["t_prod"],
            },
            Row {
                case: "producer, only a commenter bound — nobody receives",
                audience: Audience::SubjectProducer,
                bindings: vec![commenter("t_comm", &[])],
                made_by: None,
                parent_author_task_id: None,
                replies_to: vec![],
                expected: vec![],
            },
            Row {
                case: "parent's author, a reply in a thread the commenter joined — it and the producer receive",
                audience: parent_reply("evt:root"),
                bindings: vec![
                    creator("t_prod"),
                    commenter("t_comm", &["k:thread"]),
                    responder("t_resp"),
                ],
                made_by: None,
                parent_author_task_id: None,
                replies_to: keys(&["k:thread"]),
                expected: vec!["t_prod", "t_comm"],
            },
            Row {
                case: "parent's author, a reply to the commenter's own comment — it receives",
                audience: parent_reply("evt:root"),
                bindings: vec![commenter("t_comm", &[]), responder("t_resp")],
                made_by: None,
                parent_author_task_id: Some("t_comm"),
                replies_to: vec![],
                expected: vec!["t_comm"],
            },
            Row {
                case: "parent's author, a reply in another thread — the commenter is no fallback",
                audience: parent_reply("evt:root"),
                bindings: vec![commenter("t_comm", &["k:thread"]), responder("t_resp")],
                made_by: None,
                parent_author_task_id: None,
                replies_to: keys(&["k:elsewhere"]),
                expected: vec!["t_resp"],
            },
        ]
    }

    #[test]
    fn the_table_holds() {
        for row in table() {
            let got = recipients(
                &row.audience,
                &row.bindings,
                EventProvenance {
                    made_by: row.made_by,
                    parent_author_task_id: row.parent_author_task_id,
                    replies_to: &row.replies_to,
                },
            );
            assert_eq!(got, row.expected, "{}", row.case);
        }
    }

    /// The invariant across the table: an event for everyone reaches every
    /// active binding it is not itself authored by, save a commenter it does
    /// not answer. An empty result therefore means every bound task wrote the
    /// event or only commented — never that the rules quietly ate it, which is
    /// how a real review was lost.
    #[test]
    fn an_event_for_everyone_reaches_every_watching_binding_that_did_not_write_it() {
        for row in table() {
            if row.audience != Audience::Everyone {
                continue;
            }
            let not_the_author: Vec<String> = row
                .bindings
                .iter()
                .filter(|binding| binding.watching)
                .filter(|binding| row.made_by != Some(binding.task_id.as_str()))
                .filter(|binding| {
                    !hears_only_replies(binding) || holds_reply_key(binding, &row.replies_to)
                })
                .map(|binding| binding.task_id.clone())
                .collect();
            let got = recipients(
                &row.audience,
                &row.bindings,
                EventProvenance {
                    made_by: row.made_by,
                    parent_author_task_id: row.parent_author_task_id,
                    replies_to: &row.replies_to,
                },
            );
            assert_eq!(got, not_the_author, "{}", row.case);
            assert_eq!(
                got.is_empty(),
                not_the_author.is_empty(),
                "an empty result for `everyone` is only ever the author's own event, \
                 one no commenter is answered by, or one no watch hears: {}",
                row.case
            );
        }
    }

    /// What the fallbacks buy: a recipient set is empty only where every
    /// binding the audience named is excluded as the event's author, or every
    /// binding only commented, never because the rules quietly narrowed it to
    /// nobody — the property `fan_out` relies on to tell "reached nobody" from
    /// "never for anyone".
    #[test]
    fn an_empty_result_can_always_name_the_task_that_wrote_the_event() {
        for row in table() {
            let got = recipients(
                &row.audience,
                &row.bindings,
                EventProvenance {
                    made_by: row.made_by,
                    parent_author_task_id: row.parent_author_task_id,
                    replies_to: &row.replies_to,
                },
            );
            if !got.is_empty() || row.bindings.iter().all(hears_only_replies) {
                continue;
            }
            let author_is_bound = row.made_by.is_some_and(|author| {
                row.bindings
                    .iter()
                    .any(|binding| binding.task_id.as_str() == author)
            });
            assert!(
                author_is_bound,
                "a bound subject resolved to nobody without an author to blame: {}",
                row.case
            );
        }
    }

    /// Names each audience by an exhaustive match, so a fourth variant stops
    /// compiling here — the table cannot silently fall behind the enum.
    fn audience_name(audience: &Audience) -> &'static str {
        match audience {
            Audience::Everyone => "everyone",
            Audience::SubjectProducer => "the subject's producer",
            Audience::ParentAuthor { .. } => "a named parent's author",
        }
    }

    /// Every audience is enumerated against several role-and-provenance shapes
    /// rather than being represented by a single happy row.
    #[test]
    fn the_table_covers_every_audience() {
        let rows = table();
        for name in [
            "everyone",
            "the subject's producer",
            "a named parent's author",
        ] {
            let covered = rows
                .iter()
                .filter(|row| audience_name(&row.audience) == name)
                .count();
            assert!(
                covered >= 4,
                "audience `{name}` needs a row per role shape, has {covered}"
            );
        }
    }
}
