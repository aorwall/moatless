//! SQL execution belongs to persistence and database infrastructure owners.

use anyhow::Result;

use crate::workspace::Workspace;
use crate::{Violation, annotations};

pub const RULE: &str = "persistence-sql-location";

fn owner(file: &str) -> bool {
    file.ends_with("/dao.rs")
        || file.ends_with("_dao.rs")
        || file.contains("/dao/")
        || file.ends_with("/db.rs")
        || file.contains("/db/")
        || file.contains("/migrations/")
        || file.ends_with("/migrations.rs")
}

fn execution(path: &str) -> bool {
    let parts: Vec<_> = path.split("::").collect();
    parts.contains(&"sqlx")
        && parts.iter().any(|p| {
            matches!(
                *p,
                "query"
                    | "query_as"
                    | "query_scalar"
                    | "query_unchecked"
                    | "query_as_unchecked"
                    | "query_scalar_unchecked"
                    | "query_file"
                    | "query_file_as"
                    | "query_file_scalar"
                    | "raw_sql"
                    | "QueryBuilder"
            )
        })
}

pub fn check(ws: &Workspace) -> Result<Vec<Violation>> {
    let index = ws.role_index()?;
    let mut out = Vec::new();
    for r in index.records.iter().filter(|r| !r.test && !owner(&r.file)) {
        if annotations::permits(r, RULE) {
            continue;
        }
        let executes = r
            .references
            .iter()
            .any(|p| execution(&index.resolve(r, p).unwrap_or_else(|| p.join("::"))));
        if executes {
            out.push(Violation::at(RULE, &r.id, "SQL execution belongs in a DAO, database infrastructure or an explicit read projection", r.location()));
        }
    }
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "fixture rules must run")]
mod tests {
    use super::*;
    use crate::fixtures::Fixture;
    #[test]
    fn a_projection_filename_does_not_exempt_sql_execution() {
        let mut f = Fixture::new();
        f.member("t3code", "", "mod projection;");
        f.write(
            "t3code/src/projection.rs",
            "fn read() { sqlx::query(\"SELECT 1\"); }",
        );
        let found = check(&f.load()).expect("check");
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn checked_query_macros_stay_with_their_owner() {
        let mut f = Fixture::new();
        f.member("sample", "", "mod dao; mod handlers;");
        f.write(
            "sample/src/dao.rs",
            "fn read(id: i64) { sqlx::query!(\"SELECT $1::int8\", id); }",
        );
        f.write(
            "sample/src/handlers.rs",
            "fn read() { sqlx::query_as!(Row, \"SELECT 1\"); } fn count() { sqlx::query_scalar!(\"SELECT 1\"); }",
        );
        let found = check(&f.load()).expect("check");
        assert_eq!(found.len(), 2);
        assert!(
            found
                .iter()
                .all(|v| v.locations[0].contains("/handlers.rs:"))
        );
    }

    #[test]
    fn queries_and_aliases_stay_with_their_owner() {
        let mut f = Fixture::new();
        f.member("sample", "", "mod dao; mod handlers;");
        f.write(
            "sample/src/dao.rs",
            "fn read() { sqlx::query(\"SELECT 1\"); }",
        );
        f.write("sample/src/handlers.rs", "use sqlx::query as execute; fn read() { execute(\"SELECT 1\"); } #[cfg(test)] fn test() { sqlx::query(\"SELECT 1\"); }");
        let found = check(&f.load()).expect("check");
        assert!(!found.is_empty());
        assert!(
            found
                .iter()
                .all(|v| v.locations[0].contains("/handlers.rs:"))
        );
    }
}
