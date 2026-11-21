//! A migration that drops or renames a table or column names the earlier
//! release that stopped reading it; `crates/core/AGENTS.md` owns the process.

use crate::Violation;
use crate::sql;
use crate::workspace::Workspace;
use anyhow::Result;
use regex::Regex;
use std::sync::OnceLock;

pub const RULE: &str = "migration-contract";

/// The newest migration this rule does not read. Applied migrations are
/// immutable (production records their checksums), so the ones before the
/// rule cannot take a marker. Never move it.
const LAST_UNCHECKED: u32 = 248;

fn destructive(sql: &str) -> Vec<String> {
    static DDL: OnceLock<Regex> = OnceLock::new();
    let ddl = DDL.get_or_init(|| {
        Regex::new(r#"(?i)\bdrop\s+(?:table|column)\b[^;,]*|\balter\s+table\b[^;]*?\brename\s+(?:column\b|to\b|"?[a-z_][a-z0-9_]*"?\s+to\b)[^;,]*"#)
            .unwrap_or_else(|e| panic!("invalid migration matcher: {e}"))
    });
    ddl.find_iter(&sql::uncomment(sql))
        .map(|m| m.as_str().split_whitespace().collect::<Vec<_>>().join(" "))
        .collect()
}

fn has_marker(sql: &str) -> bool {
    static MARKER: OnceLock<Regex> = OnceLock::new();
    MARKER
        .get_or_init(|| {
            Regex::new(r"(?m)^\s*--\s*contract-after:\s*#\d+")
                .unwrap_or_else(|e| panic!("invalid marker matcher: {e}"))
        })
        .is_match(sql)
}

fn judge(file: &str, sql: &str) -> Option<Violation> {
    let version: u32 = file.split('_').next()?.parse().ok()?;
    if version <= LAST_UNCHECKED || has_marker(sql) {
        return None;
    }
    let found = destructive(sql);
    (!found.is_empty()).then(|| Violation {
        rule: RULE,
        key: file.to_string(),
        message: format!(
            "`{}` breaks the replica still on the previous release; ship the reader removal first and add `-- contract-after: #<that PR>` (crates/core/AGENTS.md)",
            found.join("`, `")
        ),
        locations: vec![format!("crates/core/migrations/{file}")],
    })
}

pub fn check(ws: &Workspace) -> Result<Vec<Violation>> {
    let Some(core) = ws.member("moatless-core") else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for entry in std::fs::read_dir(core.dir.join("migrations"))? {
        let path = entry?.path();
        if path.extension().is_none_or(|e| e != "sql") {
            continue;
        }
        if let Some(file) = path.file_name().and_then(|f| f.to_str()) {
            out.extend(judge(file, &std::fs::read_to_string(&path)?));
        }
    }
    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drop_or_rename_needs_the_marker() {
        for sql in [
            "ALTER TABLE tasks DROP COLUMN scope;",
            "ALTER TABLE tasks DROP COLUMN IF EXISTS site_url, DROP COLUMN vibe;",
            "DROP TABLE IF EXISTS teams;",
            "ALTER TABLE tasks RENAME COLUMN a TO b;",
            "ALTER TABLE tasks RENAME a TO b;",
            "ALTER TABLE artifact_records RENAME TO collection_records;",
        ] {
            assert!(judge("0249_x.sql", sql).is_some(), "{sql}");
            let marked = format!("-- contract-after: #1575\n{sql}");
            assert!(judge("0249_x.sql", &marked).is_none(), "{sql}");
        }
    }

    #[test]
    fn additive_sql_prose_and_old_migrations_pass() {
        assert!(judge("0249_x.sql", "-- Dropping the column\nALTER TABLE t ADD COLUMN c int; DROP INDEX i; ALTER TABLE t DROP CONSTRAINT k; ALTER INDEX i RENAME TO j; ALTER TABLE t RENAME CONSTRAINT a TO b;").is_none());
        assert!(
            judge(
                "0248_drop_task_scope.sql",
                "ALTER TABLE tasks DROP COLUMN scope;"
            )
            .is_none()
        );
    }

    #[test]
    fn a_marker_without_a_pr_number_does_not_count() {
        assert!(judge("0249_x.sql", "-- contract-after: soon\nDROP TABLE t;").is_some());
    }
}
