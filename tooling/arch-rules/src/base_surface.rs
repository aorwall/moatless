//! Adapter transport features activate their dependencies explicitly.

use crate::Violation;
use crate::annotations::permits_dependency;
use crate::workspace::Workspace;
use anyhow::Result;

pub const RULE: &str = "base-surface";
const TRANSPORTS: &[&str] = &[
    "sqlx",
    "axum",
    "tokio",
    "moatless-core",
    "moatless-api-schemas",
    "reqwest",
];

fn gated(optional: bool, dependency: &str, features: &toml::Table) -> bool {
    let activates = |v: &toml::Value| {
        v.as_array().is_some_and(|a| {
            a.iter().any(|s| {
                s.as_str()
                    .is_some_and(|s| s == dependency || s == format!("dep:{dependency}"))
            })
        })
    };
    optional
        && features
            .iter()
            .filter(|(_, values)| activates(values))
            .count()
            == 1
        && !default_activates(dependency, features)
}

fn default_activates(dependency: &str, features: &toml::Table) -> bool {
    let mut pending = vec!["default"];
    let mut seen = std::collections::BTreeSet::new();
    while let Some(feature) = pending.pop() {
        if !seen.insert(feature) {
            continue;
        }
        for value in features
            .get(feature)
            .and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(toml::Value::as_str)
        {
            if value == dependency || value == format!("dep:{dependency}") {
                return true;
            }
            pending.push(value);
        }
    }
    false
}

pub fn check(ws: &Workspace) -> Result<Vec<Violation>> {
    let mut out = Vec::new();
    if let Some(m) = ws.member("moatless-adapter-model") {
        let empty = toml::Table::new();
        let features = m
            .manifest
            .get("features")
            .and_then(toml::Value::as_table)
            .unwrap_or(&empty);
        for d in m
            .dependencies
            .iter()
            .filter(|d| d.kind.is_none() && TRANSPORTS.contains(&d.name.as_str()))
        {
            if !gated(d.optional, &d.alias, features) && !permits_dependency(m, RULE, &d.name) {
                out.push(Violation::at(
                    RULE,
                    format!("{} -> {}", m.name, d.name),
                    "transport must be optional and activated directly by one non-default feature",
                    ws.relative(&m.dir.join("Cargo.toml")),
                ));
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
    fn indirect_feature_composition_is_not_a_second_activation() {
        let features = "http = ['dep:axum']\ntest-support = ['http']"
            .parse()
            .unwrap();
        assert!(gated(true, "axum", &features));
        assert!(!gated(false, "axum", &features));
        assert!(!gated(true, "sqlx", &features));
    }
    #[test]
    fn defaults_cannot_activate_a_transport_transitively() {
        let features = "default = ['outer']\nouter = ['http']\nhttp = ['dep:axum']"
            .parse()
            .unwrap();
        assert!(!gated(true, "axum", &features));
    }
    #[test]
    fn two_direct_owners_fail() {
        let features = "http = ['dep:axum']\nother = ['dep:axum']".parse().unwrap();
        assert!(!gated(true, "axum", &features));
    }
}
