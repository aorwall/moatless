//! `AGENT_FALLBACK_ORDER`: which harnesses a failed turn may move to.

use std::collections::HashMap;

use moatless_types::agent::Harness;

/// For each harness, the harnesses a turn its provider would not serve moves
/// to, tried in order until one is installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentFallbackOrder(HashMap<Harness, Vec<Harness>>);

impl Default for AgentFallbackOrder {
    fn default() -> Self {
        use Harness::{Acp, ClaudeCode, Codex};
        Self(HashMap::from([
            (ClaudeCode, vec![Codex]),
            (Codex, vec![ClaudeCode]),
            (Acp, vec![ClaudeCode, Codex]),
        ]))
    }
}

impl AgentFallbackOrder {
    /// Where a turn that failed on `harness` may go; empty for none.
    pub fn targets(&self, harness: Harness) -> &[Harness] {
        self.0.get(&harness).map_or(&[], Vec::as_slice)
    }

    /// Parses `acp=claude-code,codex;pi=` over the default: each named
    /// harness's list replaces its default, and an empty list turns it off.
    pub fn parse(raw: &str) -> Result<Self, String> {
        let mut order = Self::default();
        for entry in raw.split(';').map(str::trim).filter(|e| !e.is_empty()) {
            let (from, to) = entry
                .split_once('=')
                .ok_or_else(|| format!("AGENT_FALLBACK_ORDER entry `{entry}` has no `=`"))?;
            let from = harness(from)?;
            let to = to
                .split(',')
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(harness)
                .collect::<Result<Vec<_>, _>>()?;
            if to.contains(&from) {
                return Err(format!(
                    "AGENT_FALLBACK_ORDER: `{from}` cannot fall back to itself"
                ));
            }
            order.0.insert(from, to);
        }
        Ok(order)
    }
}

fn harness(raw: &str) -> Result<Harness, String> {
    let raw = raw.trim();
    Harness::parse(raw).ok_or_else(|| format!("AGENT_FALLBACK_ORDER: unknown harness `{raw}`"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn antigravity_falls_back_to_claude_code_first() {
        let order = AgentFallbackOrder::default();
        assert_eq!(
            order.targets(Harness::Acp),
            [Harness::ClaudeCode, Harness::Codex]
        );
        assert_eq!(order.targets(Harness::Codex), [Harness::ClaudeCode]);
        assert!(order.targets(Harness::Pi).is_empty());
    }

    #[test]
    fn a_named_harness_replaces_its_default_and_an_empty_list_turns_it_off() {
        let order =
            AgentFallbackOrder::parse(" acp = codex ; claude-code= ; pi=claude-code").unwrap();
        assert_eq!(order.targets(Harness::Acp), [Harness::Codex]);
        assert!(order.targets(Harness::ClaudeCode).is_empty());
        assert_eq!(order.targets(Harness::Pi), [Harness::ClaudeCode]);
        assert_eq!(order.targets(Harness::Codex), [Harness::ClaudeCode]);
    }

    #[test]
    fn a_malformed_order_is_refused() {
        for raw in ["acp", "acp=gemini", "nope=codex", "codex=codex"] {
            assert!(AgentFallbackOrder::parse(raw).is_err(), "{raw}");
        }
    }
}
