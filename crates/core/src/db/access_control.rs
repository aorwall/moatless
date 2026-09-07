//! SQL predicates for the shared access policy.

use moatless_types::access_control::ScopedEntity;

fn has_visibility(entity: ScopedEntity) -> bool {
    matches!(entity, ScopedEntity::Task)
}

/// Access-control predicate and the next positional parameter index.
pub struct AccessFilter {
    sql: String,
    next_param: usize,
}

impl AccessFilter {
    /// The SQL WHERE clause fragment (e.g. `($1 OR visibility = 'public' OR ...)`).
    pub fn sql(&self) -> &str {
        &self.sql
    }

    /// Next available positional parameter index after this filter's params.
    pub fn next_param(&self) -> usize {
        self.next_param
    }
}

/// Build a read SQL filter for the given entity.
///
/// Uses 2 positional parameters starting at `start`:
///   - `$start`: `is_admin` (bool)
///   - `$start+1`: `user_id` (text)
///
/// Caller must `.bind()` these from `ScopeContext::is_admin`, `ScopeContext::user_id`.
///
/// `prefix` is the table alias (e.g. `""` or `"t."`).
/// The entity determines whether `visibility = 'public'` is included.
pub fn access_filter(entity: ScopedEntity, prefix: &str, start: usize) -> AccessFilter {
    let (p1, p2) = (start, start + 1);
    let vis = if has_visibility(entity) {
        format!(" OR {prefix}visibility = 'public'")
    } else {
        String::new()
    };
    let sql = format!(
        "(${p1}{vis} OR {prefix}scope = 'global' \
         OR ({prefix}scope = 'user' AND COALESCE({prefix}owner_user_id, {prefix}created_by) = ${p2}))"
    );
    AccessFilter {
        sql,
        next_param: start + 2,
    }
}

/// Build a write (mutation) SQL filter.
///
/// Write is the strictest grant: global rows are admin-managed and task
/// visibility never grants it, so the fragment is identical for every
/// entity and this takes no [`ScopedEntity`].
///
/// Uses 2 positional parameters starting at `start`:
///   - `$start`: `is_admin` (bool)
///   - `$start+1`: `user_id` (text)
///
/// Caller must `.bind()` these from `ScopeContext::is_admin`, `ScopeContext::user_id`.
/// `prefix` is the table alias (e.g. `""` or `"t."`).
///
/// Embedding this in a mutation's `WHERE` makes the check and the write a
/// single atomic statement; `rows_affected == 0` maps to the module's
/// `NotFound` error, matching the existence-hiding reads.
pub fn write_filter(prefix: &str, start: usize) -> AccessFilter {
    let (p1, p2) = (start, start + 1);
    let sql = format!(
        "(${p1} OR ({prefix}scope = 'user' \
         AND COALESCE({prefix}owner_user_id, {prefix}created_by) = ${p2}))"
    );
    AccessFilter {
        sql,
        next_param: start + 2,
    }
}

/// Build a control (operate) SQL filter.
///
/// Control is write plus the public-task arm. The entity decides whether
/// that arm appears, exactly as it does for [`access_filter`], so a
/// caller cannot ask for it on a table with no `visibility` column. For
/// every entity but [`ScopedEntity::Task`] this is [`write_filter`],
/// because only a Task has anything to operate.
///
/// Uses 2 positional parameters starting at `start`:
///   - `$start`: `is_admin` (bool)
///   - `$start+1`: `user_id` (text)
///
/// Caller must `.bind()` these from `ScopeContext::is_admin`, `ScopeContext::user_id`.
/// `prefix` is the table alias (e.g. `""` or `"t."`).
pub fn control_filter(entity: ScopedEntity, prefix: &str, start: usize) -> AccessFilter {
    let (p1, p2) = (start, start + 1);
    let vis = if has_visibility(entity) {
        format!(" OR {prefix}visibility = 'public'")
    } else {
        String::new()
    };
    let sql = format!(
        "(${p1}{vis} OR ({prefix}scope = 'user' \
         AND COALESCE({prefix}owner_user_id, {prefix}created_by) = ${p2}))"
    );
    AccessFilter {
        sql,
        next_param: start + 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moatless_types::access_control::ScopeContext;

    #[test]
    fn access_filter_task_includes_visibility() {
        let f = access_filter(ScopedEntity::Task, "", 1);
        assert_eq!(
            f.sql(),
            "($1 OR visibility = 'public' OR scope = 'global' \
             OR (scope = 'user' AND COALESCE(owner_user_id, created_by) = $2))"
        );
        assert_eq!(f.next_param(), 3);
    }

    #[test]
    fn access_filter_repository_excludes_visibility() {
        let f = access_filter(ScopedEntity::Repository, "", 2);
        assert_eq!(
            f.sql(),
            "($2 OR scope = 'global' \
             OR (scope = 'user' AND COALESCE(owner_user_id, created_by) = $3))"
        );
        assert_eq!(f.next_param(), 4);
    }

    #[test]
    fn access_filter_resource_no_visibility() {
        let f = access_filter(ScopedEntity::Resource, "", 1);
        assert_eq!(
            f.sql(),
            "($1 OR scope = 'global' \
             OR (scope = 'user' AND COALESCE(owner_user_id, created_by) = $2))"
        );
        assert_eq!(f.next_param(), 3);
    }

    #[test]
    fn access_filter_with_table_prefix() {
        let f = access_filter(ScopedEntity::Task, "t.", 1);
        assert_eq!(
            f.sql(),
            "($1 OR t.visibility = 'public' OR t.scope = 'global' \
             OR (t.scope = 'user' AND COALESCE(t.owner_user_id, t.created_by) = $2))"
        );
    }

    #[test]
    fn write_filter_admin_or_owner_only() {
        let f = write_filter("", 1);
        assert_eq!(
            f.sql(),
            "($1 OR (scope = 'user' AND COALESCE(owner_user_id, created_by) = $2))"
        );
        assert_eq!(f.next_param(), 3);
    }

    #[test]
    fn write_filter_with_table_prefix() {
        let f = write_filter("t.", 3);
        assert_eq!(
            f.sql(),
            "($3 OR (t.scope = 'user' AND COALESCE(t.owner_user_id, t.created_by) = $4))"
        );
        assert_eq!(f.next_param(), 5);
    }

    #[test]
    fn control_filter_task_adds_the_public_arm_to_the_write_fragment() {
        let f = control_filter(ScopedEntity::Task, "", 1);
        assert_eq!(
            f.sql(),
            "($1 OR visibility = 'public' \
             OR (scope = 'user' AND COALESCE(owner_user_id, created_by) = $2))"
        );
        assert_eq!(f.next_param(), 3);
    }

    #[test]
    fn control_filter_without_visibility_is_the_write_fragment() {
        // Only a Task has a Sandbox to operate. On every other entity the two
        // grants coincide, and the fragment must not name a column the table
        // does not have.
        for entity in [ScopedEntity::Repository, ScopedEntity::Resource] {
            assert_eq!(
                control_filter(entity, "t.", 3).sql(),
                write_filter("t.", 3).sql()
            );
        }
    }

    #[test]
    fn system_access_filter_admin_param_is_true() {
        // The filter's first positional param is `is_admin`; for the system
        // principal it binds `true`, which makes the whole disjunct match all
        // rows — identical semantics to an unscoped `SELECT ... WHERE id = $1`.
        // reason: verify the internal principal binds the admin parameter.
        #[allow(clippy::disallowed_methods)]
        let ctx = ScopeContext::system();
        assert!(ctx.is_admin);
        let f = access_filter(ScopedEntity::Task, "", 1);
        assert!(f.sql().starts_with("($1 OR"));
    }
}
