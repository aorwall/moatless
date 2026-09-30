//! A reqwest client is not built through `Default`, which sets no timeout.
//!
//! `clippy.toml` fences reqwest's inherent constructors, but a disallowed-method
//! path cannot name a trait impl's method, so `Client::default` is caught here.
//! An inferred `Default::default()` names no type and stays out of reach.

use anyhow::Result;

use crate::workspace::Workspace;
use crate::{Violation, annotations};

pub const RULE: &str = "reqwest-default";

fn builds_by_default(path: &str) -> bool {
    matches!(
        path,
        "reqwest::Client::default" | "reqwest::ClientBuilder::default"
    )
}

pub fn check(ws: &Workspace) -> Result<Vec<Violation>> {
    let index = ws.role_index()?;
    let mut out = Vec::new();
    for r in &index.records {
        if annotations::permits(r, RULE) || matches!(r.item, syn::Item::Use(_)) {
            continue;
        }
        if r.references
            .iter()
            .any(|p| builds_by_default(&index.resolve(r, p).unwrap_or_else(|| p.join("::"))))
        {
            out.push(Violation::at(
                RULE,
                &r.id,
                "build the client with `Client::builder()` and the timeouts its binary's `Config::http` holds",
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
    fn a_default_client_is_found_through_a_use_but_a_timed_one_is_not() {
        let mut f = Fixture::new();
        f.member(
            "sample",
            "reqwest = '0.13'",
            "use reqwest::Client; pub fn a() -> Client { Client::default() } \
             pub fn b() -> reqwest::ClientBuilder { reqwest::ClientBuilder::default() } \
             pub fn c() -> Client { Client::builder().timeout(t()).build().unwrap() }",
        );
        let found = check(&f.load()).expect("check");
        assert_eq!(found.len(), 2, "{found:?}");
    }
}
