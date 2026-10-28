//! Per key prefix, which record operations a reader's page may perform.
//!
//! A rule also names which of a page's writes queue a message to the Task's
//! agent. A user who can change the Task writes past the rules.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::models::Record;
use moatless_api_schemas::artifact::RecordWriter;

/// The only `rulesVersion` this server reads.
const RULES_VERSION: u32 = 1;
/// The longest record key, so the longest prefix that can match one.
const MAX_PREFIX_LEN: usize = 200;

/// A publish's record rules. The longest prefix a key starts with decides
/// for it, and a key no prefix matches allows a page no operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordRules {
    pub rules_version: u32,
    pub prefixes: BTreeMap<String, PrefixRule>,
}

/// What a page may do with the records under one prefix. An operation left
/// out is refused to every page.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrefixRule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read: Option<Permit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub create: Option<Permit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update: Option<Permit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delete: Option<Permit>,
    /// The page writes that queue a message to the Task's agent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notify: Vec<WriteOp>,
}

/// Which readers an operation is open to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Permit {
    /// Every reader who can open the page.
    Anyone,
    /// The reader whose user id is the stored record's `submitted_by`.
    Owner,
}

/// A record write, as the rules name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WriteOp {
    /// A save under a key that holds no record, or an append.
    Create,
    /// A save under a key that holds a record.
    Update,
    Delete,
}

/// Why a page's operation was refused: the prefix whose rule refused it, or
/// `None` when no prefix matches the key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Denied {
    pub prefix: Option<String>,
}

impl RecordRules {
    /// Parse and check a rules file, answering what is wrong with it.
    pub fn parse(text: &str) -> Result<Self, String> {
        let rules: Self =
            serde_json::from_str(text).map_err(|e| format!("rules are not valid: {e}"))?;
        if rules.rules_version != RULES_VERSION {
            return Err(format!(
                "rulesVersion {} is not supported; use {RULES_VERSION}",
                rules.rules_version
            ));
        }
        for (prefix, rule) in &rules.prefixes {
            check_prefix(prefix)?;
            if rule.create == Some(Permit::Owner) {
                return Err(format!(
                    "prefix {prefix:?}: create cannot be \"owner\", because a record that does not exist yet has no owner"
                ));
            }
            if let Some(op) = rule.notify.iter().find(|op| rule.permit(**op).is_none()) {
                return Err(format!(
                    "prefix {prefix:?}: notify names {:?}, which no page may perform there",
                    op.as_str()
                ));
            }
        }
        Ok(rules)
    }

    /// Whether a page of `viewer` may perform `op` under `key`; `stored` is
    /// the record there now, for an update or a delete.
    pub fn check_write(
        &self,
        op: WriteOp,
        key: &str,
        stored: Option<&Record>,
        viewer: &str,
    ) -> Result<(), Denied> {
        let Some((prefix, rule)) = self.rule_for(key) else {
            return Err(Denied { prefix: None });
        };
        let allowed = match rule.permit(op) {
            Some(Permit::Anyone) => true,
            Some(Permit::Owner) => stored.is_some_and(|r| r.submitted_by == viewer),
            None => false,
        };
        if allowed {
            Ok(())
        } else {
            Err(Denied {
                prefix: Some(prefix.to_string()),
            })
        }
    }

    /// Whether a page of `viewer` lists `record`. Under an `owner` read rule
    /// that is the viewer's own records and every record the agent wrote.
    pub fn can_read(&self, record: &Record, viewer: &str) -> bool {
        match self.rule_for(&record.key).and_then(|(_, rule)| rule.read) {
            Some(Permit::Anyone) => true,
            Some(Permit::Owner) => {
                record.submitted_by == viewer || record.writer == RecordWriter::Agent
            }
            None => false,
        }
    }

    /// Whether a page's `op` under `key` queues a message to the agent.
    pub fn notifies(&self, op: WriteOp, key: &str) -> bool {
        self.rule_for(key)
            .is_some_and(|(_, rule)| rule.notify.contains(&op))
    }

    fn rule_for(&self, key: &str) -> Option<(&str, &PrefixRule)> {
        self.prefixes
            .iter()
            .filter(|(prefix, _)| key.starts_with(prefix.as_str()))
            .max_by_key(|(prefix, _)| prefix.len())
            .map(|(prefix, rule)| (prefix.as_str(), rule))
    }
}

impl WriteOp {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Update => "update",
            Self::Delete => "delete",
        }
    }
}

impl PrefixRule {
    fn permit(&self, op: WriteOp) -> Option<Permit> {
        match op {
            WriteOp::Create => self.create,
            WriteOp::Update => self.update,
            WriteOp::Delete => self.delete,
        }
    }
}

/// `""`, or what a record key may start with: `/`-separated segments of
/// letters, digits, `-`, `_`, `.` or `:`, the last of which may be empty.
fn check_prefix(prefix: &str) -> Result<(), String> {
    let segments: Vec<&str> = prefix.split('/').collect();
    let valid = prefix.len() <= MAX_PREFIX_LEN
        && segments.iter().enumerate().all(|(i, segment)| {
            (i + 1 == segments.len() || !segment.is_empty())
                && segment
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
        });
    if valid {
        Ok(())
    } else {
        Err(format!(
            "prefix {prefix:?} must be at most {MAX_PREFIX_LEN} characters of '/'-separated segments, each letters, digits, '-', '_', '.' or ':'"
        ))
    }
}

#[cfg(test)]
#[path = "rules_tests.rs"]
mod tests;
