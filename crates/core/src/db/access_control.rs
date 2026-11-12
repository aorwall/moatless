//! SQL predicates for the shared access policy.

use moatless_types::access_control::ScopedEntity;

fn has_visibility(entity: ScopedEntity) -> bool {
    matches!(entity, ScopedEntity::Task)
}

/// The Task arms that grant read and write but not ownership: a public Task,
/// and a Task whose Reporter is the caller.
fn task_arms(entity: ScopedEntity, prefix: &str, user_param: usize) -> String {
    if has_visibility(entity) {
        format!(" OR {prefix}visibility = 'public' OR {prefix}reporter_user_id = ${user_param}")
    } else {
        String::new()
    }
}

/// Access-control predicate and the next positional parameter index.
pub struct AccessFilter {
    sql: String,
    next_param: usize,
}

impl AccessFilter {
    /// The SQL WHERE clause fragment (e.g. `($1 OR visibility = 'public' OR
    /// ...)`).
    pub fn sql(&self) -> &str {
        &self.sql
    }

    /// Next available positional parameter index after this filter's params.
    pub fn next_param(&self) -> usize {
        self.next_param
    }
}

/// Build a read SQL filter for the given entity, using 2 positional parameters
/// from `start`: `is_admin` then `user_id`, bound from `ScopeContext`.
///
/// `prefix` is the table alias (e.g. `"t."`); the entity decides whether
/// [`task_arms`] are included.
pub fn access_filter(entity: ScopedEntity, prefix: &str, start: usize) -> AccessFilter {
    let (p1, p2) = (start, start + 1);
    let vis = task_arms(entity, prefix, p2);
    let sql = format!(
        "(${p1}{vis} OR {prefix}scope = 'global' \
         OR ({prefix}scope = 'user' AND COALESCE({prefix}owner_user_id, {prefix}created_by) = ${p2}))"
    );
    AccessFilter {
        sql,
        next_param: start + 2,
    }
}

/// Build a write (mutation) SQL filter, using 2 positional parameters from
/// `start`: `is_admin` then `user_id`, bound from `ScopeContext`.
///
/// [`owner_filter`] plus [`task_arms`], which the entity gates exactly as
/// [`access_filter`] does. Embedding it in a mutation's `WHERE` makes check and
/// write atomic, with `rows_affected == 0` mapping to `NotFound`.
pub fn write_filter(entity: ScopedEntity, prefix: &str, start: usize) -> AccessFilter {
    let (p1, p2) = (start, start + 1);
    let vis = task_arms(entity, prefix, p2);
    let sql = format!(
        "(${p1}{vis} OR ({prefix}scope = 'user' \
         AND COALESCE({prefix}owner_user_id, {prefix}created_by) = ${p2}))"
    );
    AccessFilter {
        sql,
        next_param: start + 2,
    }
}

/// Build an ownership SQL filter, using 2 positional parameters from `start`:
/// `is_admin` then `user_id`, bound from `ScopeContext`.
///
/// The SQL form of `ScopeContext::is_owner`; identical for every entity, so it
/// takes no [`ScopedEntity`].
pub fn owner_filter(prefix: &str, start: usize) -> AccessFilter {
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

#[cfg(test)]
mod tests {
    use super::*;
    use moatless_types::access_control::ScopeContext;

    #[test]
    fn access_filter_task_includes_visibility() {
        let f = access_filter(ScopedEntity::Task, "", 1);
        assert_eq!(
            f.sql(),
            "($1 OR visibility = 'public' OR reporter_user_id = $2 OR scope = 'global' \
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
            "($1 OR t.visibility = 'public' OR t.reporter_user_id = $2 OR t.scope = 'global' \
             OR (t.scope = 'user' AND COALESCE(t.owner_user_id, t.created_by) = $2))"
        );
    }

    #[test]
    fn owner_filter_admin_or_owner_only() {
        let f = owner_filter("", 1);
        assert_eq!(
            f.sql(),
            "($1 OR (scope = 'user' AND COALESCE(owner_user_id, created_by) = $2))"
        );
        assert_eq!(f.next_param(), 3);
    }

    #[test]
    fn owner_filter_with_table_prefix() {
        let f = owner_filter("t.", 3);
        assert_eq!(
            f.sql(),
            "($3 OR (t.scope = 'user' AND COALESCE(t.owner_user_id, t.created_by) = $4))"
        );
        assert_eq!(f.next_param(), 5);
    }

    #[test]
    fn write_filter_task_adds_the_public_and_reporter_arms_to_the_owner_fragment() {
        let f = write_filter(ScopedEntity::Task, "", 1);
        assert_eq!(
            f.sql(),
            "($1 OR visibility = 'public' OR reporter_user_id = $2 \
             OR (scope = 'user' AND COALESCE(owner_user_id, created_by) = $2))"
        );
        assert_eq!(f.next_param(), 3);
    }

    #[test]
    fn write_filter_without_visibility_is_the_owner_fragment() {
        // Only a Task is public. On every other entity the two coincide, and
        // the fragment must not name a column the table does not have.
        for entity in [ScopedEntity::Repository, ScopedEntity::Resource] {
            assert_eq!(
                write_filter(entity, "t.", 3).sql(),
                owner_filter("t.", 3).sql()
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
