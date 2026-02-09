use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Permission mode for the Claude CLI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    Default,
    AcceptEdits,
    BypassPermissions,
    Plan,
    Delegate,
    DontAsk,
}

impl std::fmt::Display for PermissionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Default => write!(f, "default"),
            Self::AcceptEdits => write!(f, "acceptEdits"),
            Self::BypassPermissions => write!(f, "bypassPermissions"),
            Self::Plan => write!(f, "plan"),
            Self::Delegate => write!(f, "delegate"),
            Self::DontAsk => write!(f, "dontAsk"),
        }
    }
}

/// Context passed to permission handlers.
#[derive(Debug, Clone)]
pub struct PermissionContext {
    pub tool_use_id: String,
    pub agent_id: Option<String>,
    pub suggestions: Option<Vec<PermissionUpdate>>,
    pub blocked_path: Option<String>,
    pub decision_reason: Option<String>,
}

/// Result of a permission check.
#[derive(Debug, Clone)]
pub enum PermissionResult {
    Allow {
        updated_input: Option<Map<String, Value>>,
        updated_permissions: Option<Vec<PermissionUpdate>>,
    },
    Deny {
        message: String,
        interrupt: Option<bool>,
    },
}

/// Handler trait for permission decisions.
#[async_trait]
pub trait PermissionHandler: Send + Sync {
    async fn can_use_tool(
        &self,
        tool_name: &str,
        input: &Map<String, Value>,
        context: PermissionContext,
    ) -> PermissionResult;
}

/// Permission behavior for rule-based access control.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionBehavior {
    Allow,
    Deny,
    Ask,
}

/// Destination for permission updates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionUpdateDestination {
    UserSettings,
    ProjectSettings,
    LocalSettings,
    Session,
    CliArg,
}

/// A permission rule value.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRuleValue {
    pub tool_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_content: Option<String>,
}

/// Update to the permission configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PermissionUpdate {
    #[serde(rename = "addRules")]
    AddRules {
        rules: Vec<PermissionRuleValue>,
        behavior: PermissionBehavior,
        destination: PermissionUpdateDestination,
    },
    #[serde(rename = "replaceRules")]
    ReplaceRules {
        rules: Vec<PermissionRuleValue>,
        behavior: PermissionBehavior,
        destination: PermissionUpdateDestination,
    },
    #[serde(rename = "removeRules")]
    RemoveRules {
        rules: Vec<PermissionRuleValue>,
        behavior: PermissionBehavior,
        destination: PermissionUpdateDestination,
    },
    #[serde(rename = "setMode")]
    SetMode {
        mode: PermissionMode,
        destination: PermissionUpdateDestination,
    },
    #[serde(rename = "addDirectories")]
    AddDirectories {
        directories: Vec<String>,
        destination: PermissionUpdateDestination,
    },
    #[serde(rename = "removeDirectories")]
    RemoveDirectories {
        directories: Vec<String>,
        destination: PermissionUpdateDestination,
    },
}
