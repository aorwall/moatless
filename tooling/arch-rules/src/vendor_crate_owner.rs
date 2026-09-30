//! A vendor SDK is declared by the one crate that adapts it.
//!
//! Every dependency kind counts, dev and build included: a test that needs the
//! SDK reaches it through the owner's `test-support` feature, so client setup
//! is written once and SDK types stay behind the owner's surface.

use crate::workspace::Workspace;
use crate::{Violation, annotations};
use anyhow::Result;

pub const RULE: &str = "vendor-crate-owner";

/// Vendor crate patterns and the members allowed to declare them. A pattern
/// ending in `*` matches by prefix.
const OWNERS: &[(&str, &[&str])] = &[
    ("aws-sdk-s3", &["moatless-s3"]),
    ("aws-sdk-sts", &["moatless-s3"]),
    ("aws-config", &["moatless-s3"]),
    ("aws-types", &["moatless-s3"]),
    ("aws-credential-types", &["moatless-s3"]),
    ("aws-smithy-*", &["moatless-s3"]),
    ("kube", &["moatless-sandbox-k8s"]),
    ("kube-*", &["moatless-sandbox-k8s"]),
    ("k8s-openapi", &["moatless-sandbox-k8s"]),
    ("bollard", &["moatless-sandbox-docker"]),
];

pub fn check(ws: &Workspace) -> Result<Vec<Violation>> {
    let mut out = Vec::new();
    for member in ws.members() {
        for dep in &member.dependencies {
            if annotations::permits_dependency(member, RULE, &dep.name) {
                continue;
            }
            if let Some(owners) = foreign_vendor(&member.name, &dep.name) {
                out.push(Violation {
                    rule: RULE,
                    key: format!("{} -> {}", member.name, dep.name),
                    message: format!(
                        "{} declares {}, which only {} may: use the owner's API instead",
                        member.name,
                        dep.name,
                        owners.join(", ")
                    ),
                    locations: vec![ws.relative(&member.dir.join("Cargo.toml"))],
                });
            }
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

/// Whether `dependency`, a `Cargo.toml` name, is a vendor crate with an owner.
pub(crate) fn is_vendor(dependency: &str) -> bool {
    owners(dependency).is_some()
}

fn owners(dependency: &str) -> Option<&'static [&'static str]> {
    OWNERS
        .iter()
        .find(|(pattern, _)| {
            pattern
                .strip_suffix('*')
                .map_or(*pattern == dependency, |prefix| {
                    dependency.starts_with(prefix)
                })
        })
        .map(|(_, owners)| *owners)
}

/// The owners of `dependency` when it is a vendor crate `member` does not own.
fn foreign_vendor(member: &str, dependency: &str) -> Option<&'static [&'static str]> {
    owners(dependency).filter(|owners| !owners.contains(&member))
}

#[cfg(test)]
mod tests {
    use super::foreign_vendor;

    #[test]
    fn the_owner_may_declare_its_vendor_crate() {
        assert_eq!(foreign_vendor("moatless-s3", "aws-sdk-s3"), None);
        assert_eq!(foreign_vendor("moatless-sandbox-k8s", "kube"), None);
        assert_eq!(foreign_vendor("moatless-sandbox-docker", "bollard"), None);
    }

    #[test]
    fn any_other_member_may_not() {
        assert_eq!(
            foreign_vendor("moatless-sandbox", "aws-sdk-s3"),
            Some(&["moatless-s3"][..])
        );
        assert_eq!(
            foreign_vendor("moatless-backend", "k8s-openapi"),
            Some(&["moatless-sandbox-k8s"][..])
        );
    }

    #[test]
    fn a_trailing_star_matches_the_family_and_nothing_else() {
        assert!(foreign_vendor("moatless-backend", "aws-smithy-types").is_some());
        assert!(foreign_vendor("moatless-backend", "kube-runtime").is_some());
        assert_eq!(foreign_vendor("moatless-backend", "kubernetes-ish"), None);
        assert_eq!(foreign_vendor("moatless-backend", "aws-lc-rs"), None);
    }
}
