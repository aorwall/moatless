//! Which bindings receive an inbound event.
//!
//! The one place that decision is made. Before this module the rules lived once
//! inside the GitHub PR adapter and once inside the Linear adapter, disagreeing
//! with each other on threaded replies, and neither had a test of the
//! composition: both decided while reading from the database inside the loop, so
//! testing them meant standing up Postgres and nobody did.
//!
//! [`resolve_recipients`] performs no input or output. Everything it reasons
//! over is resolved by the router before the call — the active bindings on the
//! subject, the task that authored the event, and the task that authored the
//! parent event an [`Audience::ParentAuthor`] names. The redelivery-receipt
//! check happens after it returns. That is what makes the whole matrix
//! affordable to enumerate as a table.
//!
//! Identity plays no part: the resolver is never given a user identity. A task
//! is passed over only where it can be *named* as the author of what arrived.

use crate::adapters::models::{Audience, TaskBinding, ADAPTER_EVENT_BINDING_ORIGIN};

/// What the event says about who wrote it, resolved before the resolver runs.
///
/// Both fields are task ids recovered from recorded messages — a footer naming
/// a task, a claim the task recorded when it acted, or an event identifier
/// already recorded against a task. An event that carries none of them arrives
/// with both fields `None` and is treated as authored by nobody, which is the
/// case that must still reach the producing task.
#[derive(Debug, Clone, Copy)]
pub struct EventProvenance<'a> {
    /// The task that authored the event itself, when it can be named.
    pub author_task_id: Option<&'a str>,
    /// The task that authored the parent event named by
    /// [`Audience::ParentAuthor`]. `None` for every other audience, and for a
    /// parent that resolves to no task.
    pub parent_author_task_id: Option<&'a str>,
}

/// The bindings that should receive an event with this `audience`.
///
/// Order follows `bindings`, and a binding appears at most once.
///
/// Two steps, in this order and not the other. The audience **names** bindings
/// first, knowing nothing about who wrote the event; the author of the event is
/// **excluded** from that set afterwards. Excluding first would conflate two
/// different empty sets — an audience that named nobody, which is a reason to
/// fall back, and an audience that named exactly the task that wrote the event,
/// which is a reason to deliver to nobody at all.
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
                is_producer(binding)
                    || provenance.parent_author_task_id == Some(binding.task_id.as_str())
            }),
            bindings,
        ),
    };

    // A task never receives back what it is named as the author of.
    named
        .into_iter()
        .filter(|binding| provenance.author_task_id != Some(binding.task_id.as_str()))
        .collect()
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

/// A binding formed task-to-subject: the task existed first and reached out.
///
/// Stated as the negation of the responder direction, which is deliberate and
/// is the taxonomy's own rule — see the origin constants in
/// [`crate::adapters::models`]. The responder has exactly one origin, the one
/// the router writes from an inbound event; every other origin means the task
/// reached out first. Matching a list of producers instead is what went stale
/// once, when `task_owned` existed but was not declared, and every Linear
/// producer was read as a responder.
fn is_producer(binding: &TaskBinding) -> bool {
    binding.origin != ADAPTER_EVENT_BINDING_ORIGIN
}

#[cfg(test)]
mod tests {
    use super::{resolve_recipients, EventProvenance};
    use crate::adapters::models::{
        Audience, TaskBinding, ADAPTER_EVENT_BINDING_ORIGIN, CONNECTED_THREAD_BINDING_ORIGIN,
        TASK_OWNED_BINDING_ORIGIN,
    };
    use chrono::Utc;

    /// A binding is identified in these tests by its task id; every other column
    /// is scenery. `origin` is the only field the resolver reads besides
    /// `task_id`, and it carries the task's role on the subject.
    fn binding(task_id: &str, origin: &str) -> TaskBinding {
        let now = Utc::now();
        TaskBinding {
            id: format!("ts_{task_id}"),
            task_id: task_id.to_string(),
            adapter_kind: "github".to_string(),
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

    /// One row of the matrix: an audience, the roles present on the subject, and
    /// the provenance the event carries.
    struct Row {
        /// What the row demonstrates, quoted back on failure.
        case: &'static str,
        audience: Audience,
        bindings: Vec<TaskBinding>,
        author_task_id: Option<&'static str>,
        parent_author_task_id: Option<&'static str>,
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
                author_task_id: None,
                parent_author_task_id: None,
                expected: vec!["t_prod", "t_resp"],
            },
            Row {
                case: "everyone, no bindings at all — nobody receives",
                audience: Audience::Everyone,
                bindings: vec![],
                author_task_id: None,
                parent_author_task_id: None,
                expected: vec![],
            },
            Row {
                case: "everyone, the producer authored it — only the responder receives",
                audience: Audience::Everyone,
                bindings: vec![producer("t_prod"), responder("t_resp")],
                author_task_id: Some("t_prod"),
                parent_author_task_id: None,
                expected: vec!["t_resp"],
            },
            Row {
                case: "everyone, the responder authored it — only the producer receives",
                audience: Audience::Everyone,
                bindings: vec![producer("t_prod"), responder("t_resp")],
                author_task_id: Some("t_resp"),
                parent_author_task_id: None,
                expected: vec!["t_prod"],
            },
            Row {
                case: "everyone, authored by a task bound elsewhere — everyone still receives",
                audience: Audience::Everyone,
                bindings: vec![producer("t_prod"), responder("t_resp")],
                author_task_id: Some("t_stranger"),
                parent_author_task_id: None,
                expected: vec!["t_prod", "t_resp"],
            },
            Row {
                case: "everyone, only binding authored it — nobody receives",
                audience: Audience::Everyone,
                bindings: vec![producer("t_prod")],
                author_task_id: Some("t_prod"),
                parent_author_task_id: None,
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
                author_task_id: None,
                parent_author_task_id: None,
                expected: vec!["t_prod"],
            },
            Row {
                case: "producer, only a responder bound — it receives as the fallback",
                audience: Audience::SubjectProducer,
                bindings: vec![responder("t_resp"), responder("t_resp2")],
                author_task_id: None,
                parent_author_task_id: None,
                expected: vec!["t_resp", "t_resp2"],
            },
            Row {
                // Unreachable in practice and here for the shape: this audience
                // carries build results and merge conflicts, which are provider
                // signals no task authors. If one ever were authored by the
                // producer, the producer is who it was for and nobody else
                // needs it — the fallback asks whether the audience named
                // anyone, not whether anyone it named survived.
                case: "producer, the producer authored it — nobody receives, no fallback",
                audience: Audience::SubjectProducer,
                bindings: vec![producer("t_prod"), responder("t_resp")],
                author_task_id: Some("t_prod"),
                parent_author_task_id: None,
                expected: vec![],
            },
            Row {
                case: "producer, two producers bound — both receive",
                audience: Audience::SubjectProducer,
                bindings: vec![producer("t_prod"), producer("t_prod2"), responder("t_resp")],
                author_task_id: None,
                parent_author_task_id: None,
                expected: vec!["t_prod", "t_prod2"],
            },
            Row {
                case: "producer, the producer filed the subject from the CLI — it receives",
                audience: Audience::SubjectProducer,
                bindings: vec![task_owned_producer("t_prod"), responder("t_resp")],
                author_task_id: None,
                parent_author_task_id: None,
                expected: vec!["t_prod"],
            },
            Row {
                case: "producer, no bindings at all — nobody receives",
                audience: Audience::SubjectProducer,
                bindings: vec![],
                author_task_id: None,
                parent_author_task_id: None,
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
                author_task_id: None,
                parent_author_task_id: Some("t_resp"),
                expected: vec!["t_prod", "t_resp"],
            },
            Row {
                case: "parent's author, the parent is the producer — only the producer receives",
                audience: parent_reply("evt:parent"),
                bindings: vec![producer("t_prod"), responder("t_resp")],
                author_task_id: None,
                parent_author_task_id: Some("t_prod"),
                expected: vec!["t_prod"],
            },
            Row {
                case: "parent's author, the parent resolves to no task — only the producer receives",
                audience: parent_reply("evt:parent"),
                bindings: vec![producer("t_prod"), responder("t_resp")],
                author_task_id: None,
                parent_author_task_id: None,
                expected: vec!["t_prod"],
            },
            Row {
                case: "parent's author, a reply starting its own thread — only the producer receives",
                audience: Audience::ParentAuthor {
                    parent_external_event_id: None,
                },
                bindings: vec![producer("t_prod"), responder("t_resp")],
                author_task_id: None,
                parent_author_task_id: None,
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
                author_task_id: None,
                parent_author_task_id: Some("t_resp"),
                expected: vec!["t_prod", "t_resp"],
            },
            Row {
                case: "parent's author, no producer bound — the parent's author alone, the fallback does not apply",
                audience: parent_reply("evt:parent"),
                bindings: vec![responder("t_resp"), responder("t_other")],
                author_task_id: None,
                parent_author_task_id: Some("t_resp"),
                expected: vec!["t_resp"],
            },
            Row {
                case: "parent's author, the parent's author wrote the reply too — the producer receives",
                audience: parent_reply("evt:parent"),
                bindings: vec![producer("t_prod"), responder("t_resp")],
                author_task_id: Some("t_resp"),
                parent_author_task_id: Some("t_resp"),
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
                author_task_id: Some("t_prod"),
                parent_author_task_id: Some("t_prod"),
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
                author_task_id: None,
                parent_author_task_id: None,
                expected: vec!["t_resp", "t_other"],
            },
            Row {
                case: "parent's author, no producer and a reply starting its own thread — everyone bound receives as the fallback",
                audience: Audience::ParentAuthor {
                    parent_external_event_id: None,
                },
                bindings: vec![responder("t_resp"), responder("t_other")],
                author_task_id: None,
                parent_author_task_id: None,
                expected: vec!["t_resp", "t_other"],
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
                    author_task_id: row.author_task_id,
                    parent_author_task_id: row.parent_author_task_id,
                },
            );
            assert_eq!(got, row.expected, "{}", row.case);
        }
    }

    /// The invariant across the table: an event for everyone reaches every
    /// active binding it is not itself authored by. An empty result therefore
    /// means every bound task wrote the event — never that the rules quietly
    /// ate it, which is how a real review was lost.
    #[test]
    fn an_event_for_everyone_reaches_every_binding_that_did_not_write_it() {
        for row in table() {
            if row.audience != Audience::Everyone {
                continue;
            }
            let not_the_author: Vec<String> = row
                .bindings
                .iter()
                .filter(|binding| row.author_task_id != Some(binding.task_id.as_str()))
                .map(|binding| binding.task_id.clone())
                .collect();
            let got = recipients(
                &row.audience,
                &row.bindings,
                EventProvenance {
                    author_task_id: row.author_task_id,
                    parent_author_task_id: row.parent_author_task_id,
                },
            );
            assert_eq!(got, not_the_author, "{}", row.case);
            assert_eq!(
                got.is_empty(),
                not_the_author.is_empty(),
                "an empty result for `everyone` is only ever the author's own event: {}",
                row.case
            );
        }
    }

    /// What the fallbacks buy, asserted across every row rather than per
    /// audience: a recipient set is empty only where every binding the audience
    /// named is excluded as an author of the event. So an empty result can
    /// always name the task that wrote what arrived — it is never the rules
    /// quietly narrowing an event down to nobody, which is the fault this whole
    /// change exists to remove, and it is the property the router relies on to
    /// tell a delivery that reached nobody from one that was never for anyone.
    #[test]
    fn an_empty_result_can_always_name_the_task_that_wrote_the_event() {
        for row in table() {
            let got = recipients(
                &row.audience,
                &row.bindings,
                EventProvenance {
                    author_task_id: row.author_task_id,
                    parent_author_task_id: row.parent_author_task_id,
                },
            );
            if !got.is_empty() || row.bindings.is_empty() {
                continue;
            }
            let author_is_bound = row.author_task_id.is_some_and(|author| {
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
