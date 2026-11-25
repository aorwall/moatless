//! Only executable entry functions terminate their process.

use anyhow::Result;

use crate::workspace::Workspace;
use crate::{Violation, annotations};

pub const RULE: &str = "process-exit";

fn exits(path: &str) -> bool {
    matches!(
        path,
        "std::process::exit"
            | "std::process::abort"
            | "std::intrinsics::abort"
            | "libc::exit"
            | "libc::_exit"
            | "libc::abort"
    )
}

pub fn check(ws: &Workspace) -> Result<Vec<Violation>> {
    let index = ws.role_index()?;
    let roots: std::collections::BTreeSet<_> = ws
        .members()
        .flat_map(|m| &m.targets)
        .filter(|t| t.executable())
        .map(|t| ws.relative(&t.path))
        .collect();
    let mut out = Vec::new();
    for r in index.records.iter().filter(|r| !r.test) {
        if annotations::permits(r, RULE)
            || (roots.contains(&r.file)
                && matches!(&r.item, syn::Item::Fn(f) if f.sig.ident == "main"))
        {
            continue;
        }
        if matches!(r.item, syn::Item::Use(_)) {
            continue;
        }
        if r.references
            .iter()
            .any(|p| exits(&index.resolve(r, p).unwrap_or_else(|| p.join("::"))))
        {
            out.push(Violation::at(
                RULE,
                &r.id,
                "return a termination outcome to the executable entry point",
                r.location(),
            ));
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
    fn custom_binary_entry_is_allowed_but_helpers_and_libraries_are_not() {
        let mut f = Fixture::new();
        f.member("sample", "", "pub fn stop() { std::process::exit(1); }");
        f.write("sample/Cargo.toml", "[package]\nname='sample'\nversion='0.1.0'\n[[bin]]\nname='other'\npath='entry/other.rs'\n");
        f.write(
            "sample/entry/other.rs",
            "use std::process::exit as stop; fn main() { stop(0); } fn helper() { stop(1); }",
        );
        let found = check(&f.load()).expect("check");
        assert_eq!(found.len(), 2, "{found:?}");
    }
}
