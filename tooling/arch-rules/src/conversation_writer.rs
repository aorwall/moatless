//! The Projector is the only writer of the conversation read model.

use crate::workspace::Workspace;
use crate::{Violation, annotations, sql};
use anyhow::Result;

pub const RULE: &str = "conversation-writer";
const OWNER: &str = "moatless_tasks::conversation";

fn owns(module: &str) -> bool {
    module == OWNER
        || module
            .strip_prefix(OWNER)
            .is_some_and(|rest| rest.starts_with("::"))
}

pub fn check(ws: &Workspace) -> Result<Vec<Violation>> {
    let mut out = Vec::new();
    for r in &ws.index()?.records {
        if r.test || owns(&r.module) || annotations::permits(r, RULE) {
            continue;
        }
        for literal in sql::literals(&r.item) {
            for (table, write) in sql::tables(&literal) {
                if write && table.starts_with("conversation_") {
                    out.push(Violation::at(
                        RULE,
                        format!("{} -> {table}", r.id),
                        format!("{} writes {table} outside {OWNER}", r.module),
                        r.location(),
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
#[allow(clippy::expect_used, reason = "fixture rules must run")]
mod tests {
    use super::*;
    use crate::fixtures::Fixture;
    #[test]
    fn only_the_conversation_module_writes_conversation_tables() {
        let mut f = Fixture::new();
        f.member(
            "moatless-tasks",
            "",
            "pub mod conversation;\npub mod message;\n",
        );
        f.write(
            "moatless-tasks/src/conversation.rs",
            r#"pub const P: &str = "INSERT INTO conversation_messages (uuid) VALUES ($1)";"#,
        );
        f.write(
            "moatless-tasks/src/message.rs",
            r#"pub const R: &str = "SELECT uuid FROM conversation_messages";
pub const W: &str = "UPDATE conversation_activities SET kind = $1";"#,
        );
        let found = check(&f.load()).expect("check");
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0].key.contains("conversation_activities"),
            "{found:?}"
        );
    }
}
