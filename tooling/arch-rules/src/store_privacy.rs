//! Store types stay with table owners, including database-driver declarations.

use crate::workspace::Workspace;
use crate::{Violation, annotations, table_ownership::TABLES};
use anyhow::Result;

pub const RULE: &str = "store-privacy";
fn dao_path(path: &str) -> Option<String> {
    let parts: Vec<_> = path.split("::").collect();
    let end = parts.iter().position(|p| p.ends_with("Dao"))?;
    Some(parts[..=end].join("::"))
}
pub fn check(ws: &Workspace) -> Result<Vec<Violation>> {
    let index = ws.index()?;
    let mut out = Vec::new();
    for member in ws.members() {
        if !matches!(member.name.as_str(), "moatless-backend" | "moatless-t3code")
            && !TABLES.iter().any(|(_, owner)| *owner == member.name)
        {
            for dep in &member.dependencies {
                if dep.kind.is_none()
                    && dep.name == "sqlx"
                    && (!dep.optional || matches!(member.name.as_str(), "moatless-ports"))
                {
                    out.push(Violation::at(
                        RULE,
                        format!("{} -> sqlx", member.name),
                        "a crate without a table cannot declare a database driver",
                        ws.relative(&member.dir.join("Cargo.toml")),
                    ));
                }
            }
        }
    }
    for r in &index.records {
        if r.test || annotations::permits(r, RULE) {
            continue;
        }
        for p in index.paths(r) {
            if let Some(dao) = dao_path(p) {
                if dao.starts_with("moatless_")
                    && !dao.starts_with(&format!("{}::", r.package.replace('-', "_")))
                {
                    out.push(Violation::at(
                        RULE,
                        format!("{} names {dao}", r.id),
                        "name the owning domain's port instead of its store",
                        &r.file,
                    ));
                }
            }
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_store_is_named_by_the_path_up_to_its_dao() {
        assert_eq!(
            dao_path("moatless_tasks::task::dao::TaskDao::read"),
            Some("moatless_tasks::task::dao::TaskDao".into())
        );
    }
}
