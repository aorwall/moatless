//! Shared enums used across multiple API modules.

use serde::{Deserialize, Serialize};

// =============================================================================
// TaskStatus
// =============================================================================

/// Task lifecycle status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Open,
    Closed,
    Error,
}

impl TaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
            Self::Error => "error",
        }
    }
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for TaskStatus {
    type Error = String;
    fn try_from(s: &str) -> Result<Self, String> {
        match s {
            "open" => Ok(Self::Open),
            "closed" => Ok(Self::Closed),
            "error" => Ok(Self::Error),
            other => Err(format!("unknown TaskStatus: {other:?}")),
        }
    }
}

// =============================================================================
// SandboxDesiredState
// =============================================================================

/// Persisted application intent for a task's sandbox lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SandboxDesiredState {
    Running,
    Stopped,
    Removed,
}

impl SandboxDesiredState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Stopped => "stopped",
            Self::Removed => "removed",
        }
    }
}

impl std::fmt::Display for SandboxDesiredState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for SandboxDesiredState {
    type Error = String;
    fn try_from(s: &str) -> Result<Self, String> {
        match s {
            "running" => Ok(Self::Running),
            "stopped" => Ok(Self::Stopped),
            "removed" => Ok(Self::Removed),
            other => Err(format!("unknown SandboxDesiredState: {other:?}")),
        }
    }
}

// =============================================================================
// SandboxStopReason
// =============================================================================

/// Reason the persisted sandbox intent moved away from `running`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SandboxStopReason {
    User,
    IdleReaper,
    Cleanup,
    System,
    Error,
}

impl SandboxStopReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::IdleReaper => "idle_reaper",
            Self::Cleanup => "cleanup",
            Self::System => "system",
            Self::Error => "error",
        }
    }
}

impl std::fmt::Display for SandboxStopReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for SandboxStopReason {
    type Error = String;
    fn try_from(s: &str) -> Result<Self, String> {
        match s {
            "user" => Ok(Self::User),
            "idle_reaper" => Ok(Self::IdleReaper),
            "cleanup" => Ok(Self::Cleanup),
            "system" => Ok(Self::System),
            "error" => Ok(Self::Error),
            other => Err(format!("unknown SandboxStopReason: {other:?}")),
        }
    }
}

// =============================================================================
// TaskRelationType
// =============================================================================

/// Type of a directional link between two tasks. The `from` task points at the
/// `to` task via this relation (e.g. a child `created_by` its parent).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum TaskRelationType {
    /// `from` was created during the execution of `to` — i.e. `to`'s agent (or
    /// a human caller acting on behalf of `to`) explicitly spawned `from` as a
    /// child task. Currently produced only by `POST /tasks` with an explicit
    /// `parent_task_id` body field.
    CreatedBy,
    /// `from` and `to` are peers that share an external artifact (PR, issue,
    /// thread). Produced when an inbound webhook spawns `from` and discovers
    /// an earlier task `to` on the same `task_bindings` subject_key. Not a
    /// parent-child relationship.
    RelatedTo,
}

impl TaskRelationType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CreatedBy => "created_by",
            Self::RelatedTo => "related_to",
        }
    }
}

impl std::fmt::Display for TaskRelationType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for TaskRelationType {
    type Error = String;
    fn try_from(s: &str) -> Result<Self, String> {
        match s {
            "created_by" => Ok(Self::CreatedBy),
            "related_to" => Ok(Self::RelatedTo),
            other => Err(format!("unknown TaskRelationType: {other:?}")),
        }
    }
}

// =============================================================================
// Visibility
// =============================================================================

/// Task visibility level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    Public,
    Private,
}

impl Visibility {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Private => "private",
        }
    }
}

impl std::fmt::Display for Visibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for Visibility {
    type Error = String;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "public" => Ok(Self::Public),
            "private" => Ok(Self::Private),
            other => Err(format!("unknown Visibility: {other:?}")),
        }
    }
}

// =============================================================================
// Scope
// =============================================================================

/// Access scope for repositories, tasks, integrations, and credentials.
///
/// Reads: `global` is readable by every authenticated user; `user` only by
/// its owner (and admins). Control: `global` is admin-managed; `user` is
/// owner-managed. Task `visibility = 'public'` additionally grants
/// read/interact on tasks — never control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// Visible to all authenticated users; admin-managed.
    Global,
    /// Visible only to the owner.
    User,
}

impl Scope {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::User => "user",
        }
    }
}

impl std::fmt::Display for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for Scope {
    type Error = String;
    fn try_from(s: &str) -> Result<Self, String> {
        match s {
            "global" => Ok(Self::Global),
            "user" => Ok(Self::User),
            other => Err(format!("unknown Scope: {other:?}")),
        }
    }
}

// =============================================================================
// RepositoryKind
// =============================================================================

/// Kind of repository source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum RepositoryKind {
    Git,
    Remote,
    Template,
}

impl RepositoryKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Git => "git",
            Self::Remote => "remote",
            Self::Template => "template",
        }
    }
}

impl std::fmt::Display for RepositoryKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for RepositoryKind {
    type Error = String;
    fn try_from(s: &str) -> Result<Self, String> {
        match s {
            "git" => Ok(Self::Git),
            "remote" => Ok(Self::Remote),
            "template" => Ok(Self::Template),
            other => Err(format!("unknown RepositoryKind: {other:?}")),
        }
    }
}

// =============================================================================
// RepositoryProvider
// =============================================================================

/// Git hosting provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum RepositoryProvider {
    Github,
    Gitlab,
    Gitness,
}

impl RepositoryProvider {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Github => "github",
            Self::Gitlab => "gitlab",
            Self::Gitness => "gitness",
        }
    }
}

impl std::fmt::Display for RepositoryProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for RepositoryProvider {
    type Error = String;
    fn try_from(s: &str) -> Result<Self, String> {
        match s {
            "github" => Ok(Self::Github),
            "gitlab" => Ok(Self::Gitlab),
            "gitness" => Ok(Self::Gitness),
            other => Err(format!("unknown RepositoryProvider: {other:?}")),
        }
    }
}

// =============================================================================
// MessageType
// =============================================================================

/// Type of task message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum MessageType {
    Assistant,
    User,
    System,
    Result,
    Error,
}

impl MessageType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Assistant => "assistant",
            Self::User => "user",
            Self::System => "system",
            Self::Result => "result",
            Self::Error => "error",
        }
    }
}

impl std::fmt::Display for MessageType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for MessageType {
    type Error = String;
    fn try_from(s: &str) -> Result<Self, String> {
        match s {
            "assistant" => Ok(Self::Assistant),
            "user" => Ok(Self::User),
            "system" => Ok(Self::System),
            "result" => Ok(Self::Result),
            "error" => Ok(Self::Error),
            other => Err(format!("unknown MessageType: {other:?}")),
        }
    }
}

// =============================================================================
// MessageRole
// =============================================================================

/// Role of the message sender.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
    System,
}

impl MessageRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::System => "system",
        }
    }
}

impl std::fmt::Display for MessageRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for MessageRole {
    type Error = String;
    fn try_from(s: &str) -> Result<Self, String> {
        match s {
            "user" => Ok(Self::User),
            "assistant" => Ok(Self::Assistant),
            "system" => Ok(Self::System),
            other => Err(format!("unknown MessageRole: {other:?}")),
        }
    }
}

// =============================================================================
// UserRole
// =============================================================================

/// Role assigned to a user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum UserRole {
    User,
    Admin,
}

impl UserRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Admin => "admin",
        }
    }
}

impl std::fmt::Display for UserRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for UserRole {
    type Error = String;
    fn try_from(s: &str) -> Result<Self, String> {
        match s {
            "user" => Ok(Self::User),
            "admin" => Ok(Self::Admin),
            other => Err(format!("unknown UserRole: {other:?}")),
        }
    }
}

// =============================================================================
// RoutingMode
// =============================================================================

/// Loop routing mode for message-to-task mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum RoutingMode {
    Ongoing,
    BySubject,
}

impl RoutingMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ongoing => "ongoing",
            Self::BySubject => "by_subject",
        }
    }
}

impl std::fmt::Display for RoutingMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for RoutingMode {
    type Error = String;
    fn try_from(s: &str) -> Result<Self, String> {
        match s {
            "ongoing" => Ok(Self::Ongoing),
            "by_subject" => Ok(Self::BySubject),
            other => Err(format!("unknown RoutingMode: {other:?}")),
        }
    }
}

// =============================================================================
// AdapterKind
// =============================================================================

/// Canonical external adapter identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum AdapterKind {
    Slack,
    Telegram,
    Github,
    GithubPr,
    Linear,
    Webhook,
}

impl AdapterKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Slack => "slack",
            Self::Telegram => "telegram",
            Self::Github => "github",
            Self::GithubPr => "github_pr",
            Self::Linear => "linear",
            Self::Webhook => "webhook",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "slack" => Some(Self::Slack),
            "telegram" => Some(Self::Telegram),
            "github" => Some(Self::Github),
            "github_pr" => Some(Self::GithubPr),
            "linear" => Some(Self::Linear),
            "webhook" => Some(Self::Webhook),
            _ => None,
        }
    }
}

impl std::fmt::Display for AdapterKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_status_serde_roundtrip() {
        assert_eq!(
            serde_json::to_string(&TaskStatus::Open).expect("serialize"),
            "\"open\""
        );
        assert_eq!(
            serde_json::to_string(&TaskStatus::Closed).expect("serialize"),
            "\"closed\""
        );
        assert_eq!(
            serde_json::to_string(&TaskStatus::Error).expect("serialize"),
            "\"error\""
        );

        let parsed: TaskStatus = serde_json::from_str("\"open\"").expect("deserialize");
        assert_eq!(parsed, TaskStatus::Open);
    }

    #[test]
    fn task_status_try_from() {
        assert_eq!(TaskStatus::try_from("open"), Ok(TaskStatus::Open));
        assert!(TaskStatus::try_from("invalid").is_err());
    }

    #[test]
    fn task_relation_type_serde_roundtrip() {
        assert_eq!(
            serde_json::to_string(&TaskRelationType::CreatedBy).expect("serialize"),
            "\"created_by\""
        );
        assert_eq!(
            serde_json::to_string(&TaskRelationType::RelatedTo).expect("serialize"),
            "\"related_to\""
        );

        let parsed: TaskRelationType = serde_json::from_str("\"related_to\"").expect("deserialize");
        assert_eq!(parsed, TaskRelationType::RelatedTo);
    }

    #[test]
    fn task_relation_type_try_from() {
        assert_eq!(
            TaskRelationType::try_from("created_by"),
            Ok(TaskRelationType::CreatedBy)
        );
        assert_eq!(
            TaskRelationType::try_from("related_to"),
            Ok(TaskRelationType::RelatedTo)
        );
        assert!(TaskRelationType::try_from("invalid").is_err());
    }

    #[test]
    fn visibility_serde_roundtrip() {
        assert_eq!(
            serde_json::to_string(&Visibility::Public).expect("serialize"),
            "\"public\""
        );
        assert_eq!(
            serde_json::to_string(&Visibility::Private).expect("serialize"),
            "\"private\""
        );

        let parsed: Visibility = serde_json::from_str("\"private\"").expect("deserialize");
        assert_eq!(parsed, Visibility::Private);
    }

    #[test]
    fn visibility_try_from() {
        assert_eq!(Visibility::try_from("public"), Ok(Visibility::Public));
        assert!(Visibility::try_from("invalid").is_err());
    }

    #[test]
    fn scope_serde_roundtrip() {
        assert_eq!(
            serde_json::to_string(&Scope::Global).expect("serialize"),
            "\"global\""
        );
        assert_eq!(
            serde_json::to_string(&Scope::User).expect("serialize"),
            "\"user\""
        );

        let parsed: Scope = serde_json::from_str("\"user\"").expect("deserialize");
        assert_eq!(parsed, Scope::User);
        assert!(serde_json::from_str::<Scope>("\"team\"").is_err());
    }

    #[test]
    fn scope_try_from() {
        assert_eq!(Scope::try_from("global"), Ok(Scope::Global));
        assert_eq!(Scope::try_from("user"), Ok(Scope::User));
        assert!(Scope::try_from("team").is_err());
        assert!(Scope::try_from("invalid").is_err());
    }

    #[test]
    fn repository_kind_serde_roundtrip() {
        assert_eq!(
            serde_json::to_string(&RepositoryKind::Git).expect("serialize"),
            "\"git\""
        );
        assert_eq!(
            serde_json::to_string(&RepositoryKind::Remote).expect("serialize"),
            "\"remote\""
        );
        assert_eq!(
            serde_json::to_string(&RepositoryKind::Template).expect("serialize"),
            "\"template\""
        );

        let parsed: RepositoryKind = serde_json::from_str("\"git\"").expect("deserialize");
        assert_eq!(parsed, RepositoryKind::Git);
    }

    #[test]
    fn repository_kind_try_from() {
        assert_eq!(RepositoryKind::try_from("git"), Ok(RepositoryKind::Git));
        assert_eq!(
            RepositoryKind::try_from("remote"),
            Ok(RepositoryKind::Remote)
        );
        assert_eq!(
            RepositoryKind::try_from("template"),
            Ok(RepositoryKind::Template)
        );
        assert!(RepositoryKind::try_from("invalid").is_err());
    }

    #[test]
    fn repository_provider_serde_roundtrip() {
        assert_eq!(
            serde_json::to_string(&RepositoryProvider::Github).expect("serialize"),
            "\"github\""
        );
        assert_eq!(
            serde_json::to_string(&RepositoryProvider::Gitlab).expect("serialize"),
            "\"gitlab\""
        );
        assert_eq!(
            serde_json::to_string(&RepositoryProvider::Gitness).expect("serialize"),
            "\"gitness\""
        );

        let parsed: RepositoryProvider = serde_json::from_str("\"github\"").expect("deserialize");
        assert_eq!(parsed, RepositoryProvider::Github);
    }

    #[test]
    fn repository_provider_try_from() {
        assert_eq!(
            RepositoryProvider::try_from("github"),
            Ok(RepositoryProvider::Github)
        );
        assert_eq!(
            RepositoryProvider::try_from("gitlab"),
            Ok(RepositoryProvider::Gitlab)
        );
        assert_eq!(
            RepositoryProvider::try_from("gitness"),
            Ok(RepositoryProvider::Gitness)
        );
        assert!(RepositoryProvider::try_from("invalid").is_err());
    }

    #[test]
    fn message_type_serde_roundtrip() {
        assert_eq!(
            serde_json::to_string(&MessageType::Assistant).expect("serialize"),
            "\"assistant\""
        );
        assert_eq!(
            serde_json::to_string(&MessageType::User).expect("serialize"),
            "\"user\""
        );
        assert_eq!(
            serde_json::to_string(&MessageType::System).expect("serialize"),
            "\"system\""
        );
        assert_eq!(
            serde_json::to_string(&MessageType::Result).expect("serialize"),
            "\"result\""
        );
        assert_eq!(
            serde_json::to_string(&MessageType::Error).expect("serialize"),
            "\"error\""
        );

        let parsed: MessageType = serde_json::from_str("\"assistant\"").expect("deserialize");
        assert_eq!(parsed, MessageType::Assistant);
    }

    #[test]
    fn message_type_try_from() {
        assert_eq!(
            MessageType::try_from("assistant"),
            Ok(MessageType::Assistant)
        );
        assert_eq!(MessageType::try_from("user"), Ok(MessageType::User));
        assert_eq!(MessageType::try_from("system"), Ok(MessageType::System));
        assert_eq!(MessageType::try_from("result"), Ok(MessageType::Result));
        assert_eq!(MessageType::try_from("error"), Ok(MessageType::Error));
        assert!(MessageType::try_from("invalid").is_err());
    }

    #[test]
    fn message_role_serde_roundtrip() {
        assert_eq!(
            serde_json::to_string(&MessageRole::User).expect("serialize"),
            "\"user\""
        );
        assert_eq!(
            serde_json::to_string(&MessageRole::Assistant).expect("serialize"),
            "\"assistant\""
        );
        assert_eq!(
            serde_json::to_string(&MessageRole::System).expect("serialize"),
            "\"system\""
        );

        let parsed: MessageRole = serde_json::from_str("\"user\"").expect("deserialize");
        assert_eq!(parsed, MessageRole::User);
    }

    #[test]
    fn message_role_try_from() {
        assert_eq!(MessageRole::try_from("user"), Ok(MessageRole::User));
        assert_eq!(
            MessageRole::try_from("assistant"),
            Ok(MessageRole::Assistant)
        );
        assert_eq!(MessageRole::try_from("system"), Ok(MessageRole::System));
        assert!(MessageRole::try_from("invalid").is_err());
    }

    #[test]
    fn user_role_serde_roundtrip() {
        assert_eq!(
            serde_json::to_string(&UserRole::User).expect("serialize"),
            "\"user\""
        );
        assert_eq!(
            serde_json::to_string(&UserRole::Admin).expect("serialize"),
            "\"admin\""
        );

        let parsed: UserRole = serde_json::from_str("\"admin\"").expect("deserialize");
        assert_eq!(parsed, UserRole::Admin);
    }

    #[test]
    fn user_role_try_from() {
        assert_eq!(UserRole::try_from("user"), Ok(UserRole::User));
        assert_eq!(UserRole::try_from("admin"), Ok(UserRole::Admin));
        assert!(UserRole::try_from("invalid").is_err());
    }

    #[test]
    fn routing_mode_serde_roundtrip() {
        assert_eq!(
            serde_json::to_string(&RoutingMode::Ongoing).expect("serialize"),
            "\"ongoing\""
        );
        assert_eq!(
            serde_json::to_string(&RoutingMode::BySubject).expect("serialize"),
            "\"by_subject\""
        );

        let parsed: RoutingMode = serde_json::from_str("\"by_subject\"").expect("deserialize");
        assert_eq!(parsed, RoutingMode::BySubject);
    }

    #[test]
    fn routing_mode_try_from() {
        assert_eq!(RoutingMode::try_from("ongoing"), Ok(RoutingMode::Ongoing));
        assert_eq!(
            RoutingMode::try_from("by_subject"),
            Ok(RoutingMode::BySubject)
        );
        assert!(RoutingMode::try_from("per_message").is_err());
        assert!(RoutingMode::try_from("thread_based").is_err());
        assert!(RoutingMode::try_from("invalid").is_err());
    }

    #[test]
    fn adapter_kind_linear_roundtrip() {
        assert_eq!(AdapterKind::Linear.as_str(), "linear");
        assert_eq!(AdapterKind::parse("linear"), Some(AdapterKind::Linear));
    }
}
