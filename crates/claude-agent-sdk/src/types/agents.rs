use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Definition of a custom agent that can be passed to the Claude CLI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDefinition {
    /// Description of when to use this agent.
    pub description: String,
    /// System prompt for the agent.
    pub prompt: String,
    /// Allowed tool names (inherits all if omitted).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,
    /// Explicitly disallowed tool names.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disallowed_tools: Option<Vec<String>>,
    /// Model to use for this agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<AgentModel>,
    /// MCP server configurations for this agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_servers: Option<Vec<Value>>,
    /// Skill names to preload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skills: Option<Vec<String>>,
    /// Maximum agentic turns.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
}

/// Model selection for an agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentModel {
    Sonnet,
    Opus,
    Haiku,
    Inherit,
}
