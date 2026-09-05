//! SQL row decoding belongs to a crate that declares its database driver.

use crate::Violation;
use crate::annotations;
use crate::source_index::attrs;
use crate::workspace::Workspace;
use anyhow::Result;

pub const RULE: &str = "row-stays-with-query";

fn from_row(item: &syn::Item) -> bool {
    if let syn::Item::Impl(i) = item {
        if i.trait_
            .as_ref()
            .is_some_and(|(_, p, _)| p.segments.last().is_some_and(|s| s.ident == "FromRow"))
        {
            return true;
        }
    }
    attrs(item).iter().any(|a| match &a.meta {
        syn::Meta::List(l) => l
            .tokens
            .to_string()
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .any(|w| w == "FromRow"),
        _ => false,
    })
}

pub fn check(ws: &Workspace) -> Result<Vec<Violation>> {
    Ok(ws
        .index()?
        .records
        .iter()
        .filter(|r| {
            !r.test
                && from_row(&r.item)
                && ws
                    .member(&r.package)
                    .is_some_and(|m| !m.normal_deps.contains("sqlx"))
                && !annotations::permits(r, RULE)
        })
        .map(|r| {
            Violation::at(
                RULE,
                &r.id,
                "row decoding requires a declared sqlx dependency",
                &r.file,
            )
        })
        .collect())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "fixtures must parse")]
mod tests {
    use super::*;
    #[test]
    fn manual_and_conditional_decoders_are_rows() {
        for s in [
            "#[derive(sqlx::FromRow)] struct Row {}",
            "#[cfg_attr(feature = \"db\", derive(sqlx::FromRow))] struct Row {}",
            "impl sqlx::FromRow<'_, PgRow> for Row {}",
        ] {
            assert!(from_row(&syn::parse_str(s).unwrap()));
        }
        assert!(!from_row(&syn::parse_str("struct FromRow {}").unwrap()));
    }
}
