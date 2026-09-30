//! Explicit exceptions tied to one declaration or one dependency edge.

use anyhow::Result;

use crate::source_index::{Record, attrs, doc_value};
use crate::workspace::{Member, Workspace};
use crate::{RULES, Violation};

pub const RULE: &str = "arch-allow-names-a-rule";

/// A parsed item exception.
struct Allow<'a> {
    pub rule: &'a str,
    pub reason: &'a str,
}

fn parse(line: &str) -> Result<Option<Allow<'_>>, &'static str> {
    let line = line.trim();
    if !line.starts_with("arch-allow") {
        return Ok(None);
    }
    let rest = line
        .strip_prefix("arch-allow(")
        .ok_or("expected arch-allow(rule): reason")?;
    let (rule, reason) = rest
        .split_once("):")
        .ok_or("expected arch-allow(rule): reason")?;
    let rule = rule.trim();
    let reason = reason.trim();
    if !RULES.iter().any(|r| r.name == rule) {
        return Err("unknown architecture rule");
    }
    if reason.is_empty() {
        return Err("an exception requires a reason");
    }
    if dependency_rule(rule) {
        return Err("dependency exceptions belong in package.metadata.arch.allow");
    }
    Ok(Some(Allow { rule, reason }))
}

fn dependency_rule(rule: &str) -> bool {
    matches!(
        rule,
        "band-order"
            | "dependency-privacy"
            | "closed-dependency-list"
            | "base-surface"
            | "vendor-crate-owner"
    )
}

/// Whether this declaration carries a valid exception to an item rule.
pub fn permits(record: &Record, rule: &str) -> bool {
    attrs(&record.item)
        .iter()
        .filter(|a| a.path().is_ident("doc"))
        .filter_map(doc_value)
        .any(|line| {
            parse(&line)
                .ok()
                .flatten()
                .is_some_and(|a| a.rule == rule && !a.reason.is_empty())
        })
}

/// Whether a manifest excuses exactly this rule and target package.
pub fn permits_dependency(member: &Member, rule: &str, dependency: &str) -> bool {
    member.metadata["arch"]["allow"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|a| {
            a["rule"].as_str() == Some(rule)
                && a["dependency"].as_str() == Some(dependency)
                && a["reason"].as_str().is_some_and(|r| !r.trim().is_empty())
                && dependency_rule(rule)
        })
}

/// Whether `member` declares `dependency` in a kind `rule` checks: only
/// `vendor-crate-owner` reads dev and build edges.
fn declares(member: &Member, rule: &str, dependency: &str) -> bool {
    if rule == crate::vendor_crate_owner::RULE {
        member.dependencies.iter().any(|d| d.name == dependency)
    } else {
        member.normal_deps.contains(dependency)
    }
}

/// Whether an item exception to `rule` may sit on this declaration.
fn applicable(rule: &str, r: &Record) -> bool {
    match rule {
        "port-admission" => matches!(
            r.package.as_str(),
            "moatless-ports" | "moatless-workspace-ports"
        ),
        "floor-type-admission" => matches!(
            r.package.as_str(),
            "moatless-types" | "moatless-workspace-types"
        ),
        "single-implementation-trait" => matches!(r.item, syn::Item::Trait(_)),
        // Deliberately not an `impl` or `fn`: that would excuse every constant nested in it.
        "duplicate-constant-value" => {
            matches!(r.item, syn::Item::Const(_) | syn::Item::Static(_))
        }
        "composition-root"
        | "declaration-only-roots"
        | "route-registration"
        | "transport-direction"
        | "persistence-sql-location"
        | "process-exit"
        | "env-read-owner"
        | "store-privacy"
        | "table-ownership"
        | "adapter-boundaries"
        | "row-stays-with-query" => true,
        _ => false,
    }
}

pub fn check(ws: &Workspace) -> Result<Vec<Violation>> {
    let mut out = Vec::new();
    for r in &ws.role_index()?.records {
        for line in attrs(&r.item)
            .iter()
            .filter(|a| a.path().is_ident("doc"))
            .filter_map(doc_value)
        {
            match parse(&line) {
                Err(why) => out.push(Violation::at(
                    RULE,
                    format!("{}: {}", r.id, line.trim()),
                    why,
                    &r.file,
                )),
                Ok(Some(a)) => {
                    if !applicable(a.rule, r) {
                        out.push(Violation::at(
                            RULE,
                            format!("{}: {}", r.id, a.rule),
                            "exception is not applicable to this item",
                            &r.file,
                        ));
                    }
                }
                Ok(None) => {}
            }
        }
    }
    for member in ws.members() {
        if let Some(allows) = member.metadata.get("arch").and_then(|a| a.get("allow")) {
            let Some(allows) = allows.as_array() else {
                out.push(Violation::at(
                    RULE,
                    &member.name,
                    "arch.allow must be an array of tables",
                    ws.relative(&member.dir.join("Cargo.toml")),
                ));
                continue;
            };
            for a in allows {
                let rule = a["rule"].as_str().unwrap_or_default();
                let dependency = a["dependency"].as_str().unwrap_or_default();
                if !RULES.iter().any(|r| r.name == rule)
                    || !dependency_rule(rule)
                    || !declares(member, rule, dependency)
                    || a["reason"].as_str().is_none_or(|r| r.trim().is_empty())
                {
                    out.push(Violation::at(RULE, format!("{}: {rule} -> {dependency}", member.name),
                        "dependency exception requires a registered dependency rule, a declared target and a reason",
                        ws.relative(&member.dir.join("Cargo.toml"))));
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "fixtures must parse")]
mod tests {
    use super::*;
    #[test]
    fn malformed_empty_and_unknown_tags_fail() {
        for line in [
            "arch-allow(unknown): why",
            "arch-allow(port-admission): ",
            "arch-allow(port-admission)",
        ] {
            assert!(parse(line).is_err(), "{line}");
        }
    }
    #[test]
    fn item_exception_keeps_the_reason() {
        let a = parse(" arch-allow(port-admission): keeps the read projection independent")
            .unwrap()
            .unwrap();
        assert_eq!(a.rule, "port-admission");
        assert_eq!(a.reason, "keeps the read projection independent");
        assert!(parse("ordinary documentation").unwrap().is_none());
    }
    #[test]
    fn a_doc_tag_cannot_excuse_a_cargo_edge() {
        assert!(parse("arch-allow(band-order): deliberate").is_err());
    }

    fn record(src: &str) -> Record {
        Record {
            id: "moatless-core::X".to_string(),
            package: "moatless-core".to_string(),
            module: "moatless_core".to_string(),
            file: "crates/core/src/lib.rs".to_string(),
            test: false,
            exported: true,
            item: syn::parse_str(src).unwrap(),
            references: Vec::new(),
            imports: Vec::new(),
        }
    }

    #[test]
    fn a_duplicate_constant_exception_sits_on_the_constant_only() {
        let rule = crate::duplicate_constant_value::RULE;
        let tag = "/// arch-allow(duplicate-constant-value): a wire literal both ends pin\n";
        let constant = record(&format!("{tag}pub const A: &str = \"shared value\";"));
        assert!(permits(&constant, rule));
        assert!(applicable(rule, &constant));
        assert!(applicable(
            rule,
            &record(&format!("{tag}static A: u64 = 9000;"))
        ));
        assert!(!applicable(rule, &record(&format!("{tag}impl X {{}}"))));
    }
}
