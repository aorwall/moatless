//! The conversation's deterministic ids (plan 4.9): `UUIDv5` in one namespace,
//! over a name that starts with the Task holding the row, so a rebuilt or
//! inherited conversation gets the same ids and a fork's never equal its
//! source's.

use uuid::Uuid;

/// Moatless's conversation namespace. Changing it renames every live row.
const NAMESPACE: Uuid = Uuid::from_u128(0x5a68_3ef5_fd17_46dd_a889_cbee_5fda_e850);

/// A Message's id, from the source row that opened it: an inherited inbox
/// item, the first line of a folded run, or a task event.
pub(crate) fn message(task_id: &str, source: &str) -> Uuid {
    Uuid::new_v5(&NAMESPACE, format!("message:{task_id}:{source}").as_bytes())
}

/// An activity's id, from `name`: `{task_id}:{source}` with the source's own
/// suffix (the line-local block index, a joined row's kind).
pub(crate) fn activity(name: &str) -> Uuid {
    Uuid::new_v5(&NAMESPACE, format!("activity:{name}").as_bytes())
}

/// The activity name of a source with no block of its own: a task event or an
/// inbox item, and `suffix` for a row it joins.
pub(crate) fn scope(task_id: &str, source: &str) -> String {
    format!("{task_id}:{source}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_version_5_and_scoped_by_task() {
        let own = message("task-a", "evt-1");
        assert_eq!(own.get_version_num(), 5);
        assert_eq!(own, message("task-a", "evt-1"));
        assert_ne!(own, message("task-b", "evt-1"));
        assert_ne!(activity("task-a:evt-1:0"), activity("task-b:evt-1:0"));
    }

    /// Computed outside Rust (Python's `uuid.uuid5`), so a changed namespace
    /// or name format fails here rather than renaming every live row.
    #[test]
    fn ids_match_their_reference_values() {
        assert_eq!(
            message("task-a", "evt-1").to_string(),
            "11877f2d-8cf3-5316-8d9e-ec9e739c417f"
        );
        assert_eq!(
            activity("task-a:evt-1:0").to_string(),
            "c856b018-2712-58f6-815d-e37b93c9749b"
        );
    }
}
