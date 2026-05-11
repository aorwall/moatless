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

#[cfg(feature = "database")]
impl sqlx::Type<sqlx::Postgres> for TaskStatus {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <String as sqlx::Type<sqlx::Postgres>>::type_info()
    }
}

#[cfg(feature = "database")]
impl<'r> sqlx::Decode<'r, sqlx::Postgres> for TaskStatus {
    fn decode(value: sqlx::postgres::PgValueRef<'r>) -> Result<Self, sqlx::error::BoxDynError> {
        let s = <String as sqlx::Decode<sqlx::Postgres>>::decode(value)?;
        TaskStatus::try_from(s.as_str()).map_err(Into::into)
    }
}

#[cfg(feature = "database")]
impl sqlx::Encode<'_, sqlx::Postgres> for TaskStatus {
    fn encode_by_ref(
        &self,
        buf: &mut sqlx::postgres::PgArgumentBuffer,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        <&str as sqlx::Encode<sqlx::Postgres>>::encode_by_ref(&self.as_str(), buf)
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

#[cfg(feature = "database")]
impl sqlx::Type<sqlx::Postgres> for Visibility {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <String as sqlx::Type<sqlx::Postgres>>::type_info()
    }
}

#[cfg(feature = "database")]
impl<'r> sqlx::Decode<'r, sqlx::Postgres> for Visibility {
    fn decode(value: sqlx::postgres::PgValueRef<'r>) -> Result<Self, sqlx::error::BoxDynError> {
        let s = <String as sqlx::Decode<sqlx::Postgres>>::decode(value)?;
        Visibility::try_from(s.as_str()).map_err(Into::into)
    }
}

#[cfg(feature = "database")]
impl sqlx::Encode<'_, sqlx::Postgres> for Visibility {
    fn encode_by_ref(
        &self,
        buf: &mut sqlx::postgres::PgArgumentBuffer,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        <&str as sqlx::Encode<sqlx::Postgres>>::encode_by_ref(&self.as_str(), buf)
    }
}

// =============================================================================
// Scope
// =============================================================================

/// 3-level scope for repositories, tasks, integrations, and credentials.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// Visible to all authenticated users.
    Global,
    /// Visible to team members.
    Team,
    /// Visible only to the creator.
    User,
}

impl Scope {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Team => "team",
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
            "team" => Ok(Self::Team),
            "user" => Ok(Self::User),
            other => Err(format!("unknown Scope: {other:?}")),
        }
    }
}

#[cfg(feature = "database")]
impl sqlx::Type<sqlx::Postgres> for Scope {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <String as sqlx::Type<sqlx::Postgres>>::type_info()
    }
}

#[cfg(feature = "database")]
impl<'r> sqlx::Decode<'r, sqlx::Postgres> for Scope {
    fn decode(value: sqlx::postgres::PgValueRef<'r>) -> Result<Self, sqlx::error::BoxDynError> {
        let s = <String as sqlx::Decode<sqlx::Postgres>>::decode(value)?;
        Scope::try_from(s.as_str()).map_err(Into::into)
    }
}

#[cfg(feature = "database")]
impl sqlx::Encode<'_, sqlx::Postgres> for Scope {
    fn encode_by_ref(
        &self,
        buf: &mut sqlx::postgres::PgArgumentBuffer,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        <&str as sqlx::Encode<sqlx::Postgres>>::encode_by_ref(&self.as_str(), buf)
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

#[cfg(feature = "database")]
impl sqlx::Type<sqlx::Postgres> for RepositoryKind {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <String as sqlx::Type<sqlx::Postgres>>::type_info()
    }
}

#[cfg(feature = "database")]
impl<'r> sqlx::Decode<'r, sqlx::Postgres> for RepositoryKind {
    fn decode(value: sqlx::postgres::PgValueRef<'r>) -> Result<Self, sqlx::error::BoxDynError> {
        let s = <String as sqlx::Decode<sqlx::Postgres>>::decode(value)?;
        RepositoryKind::try_from(s.as_str()).map_err(Into::into)
    }
}

#[cfg(feature = "database")]
impl sqlx::Encode<'_, sqlx::Postgres> for RepositoryKind {
    fn encode_by_ref(
        &self,
        buf: &mut sqlx::postgres::PgArgumentBuffer,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        <&str as sqlx::Encode<sqlx::Postgres>>::encode_by_ref(&self.as_str(), buf)
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

#[cfg(feature = "database")]
impl sqlx::Type<sqlx::Postgres> for RepositoryProvider {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <String as sqlx::Type<sqlx::Postgres>>::type_info()
    }
}

#[cfg(feature = "database")]
impl<'r> sqlx::Decode<'r, sqlx::Postgres> for RepositoryProvider {
    fn decode(value: sqlx::postgres::PgValueRef<'r>) -> Result<Self, sqlx::error::BoxDynError> {
        let s = <String as sqlx::Decode<sqlx::Postgres>>::decode(value)?;
        RepositoryProvider::try_from(s.as_str()).map_err(Into::into)
    }
}

#[cfg(feature = "database")]
impl sqlx::Encode<'_, sqlx::Postgres> for RepositoryProvider {
    fn encode_by_ref(
        &self,
        buf: &mut sqlx::postgres::PgArgumentBuffer,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        <&str as sqlx::Encode<sqlx::Postgres>>::encode_by_ref(&self.as_str(), buf)
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

#[cfg(feature = "database")]
impl sqlx::Type<sqlx::Postgres> for MessageType {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <String as sqlx::Type<sqlx::Postgres>>::type_info()
    }
}

#[cfg(feature = "database")]
impl<'r> sqlx::Decode<'r, sqlx::Postgres> for MessageType {
    fn decode(value: sqlx::postgres::PgValueRef<'r>) -> Result<Self, sqlx::error::BoxDynError> {
        let s = <String as sqlx::Decode<sqlx::Postgres>>::decode(value)?;
        MessageType::try_from(s.as_str()).map_err(Into::into)
    }
}

#[cfg(feature = "database")]
impl sqlx::Encode<'_, sqlx::Postgres> for MessageType {
    fn encode_by_ref(
        &self,
        buf: &mut sqlx::postgres::PgArgumentBuffer,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        <&str as sqlx::Encode<sqlx::Postgres>>::encode_by_ref(&self.as_str(), buf)
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

#[cfg(feature = "database")]
impl sqlx::Type<sqlx::Postgres> for MessageRole {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <String as sqlx::Type<sqlx::Postgres>>::type_info()
    }
}

#[cfg(feature = "database")]
impl<'r> sqlx::Decode<'r, sqlx::Postgres> for MessageRole {
    fn decode(value: sqlx::postgres::PgValueRef<'r>) -> Result<Self, sqlx::error::BoxDynError> {
        let s = <String as sqlx::Decode<sqlx::Postgres>>::decode(value)?;
        MessageRole::try_from(s.as_str()).map_err(Into::into)
    }
}

#[cfg(feature = "database")]
impl sqlx::Encode<'_, sqlx::Postgres> for MessageRole {
    fn encode_by_ref(
        &self,
        buf: &mut sqlx::postgres::PgArgumentBuffer,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        <&str as sqlx::Encode<sqlx::Postgres>>::encode_by_ref(&self.as_str(), buf)
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

#[cfg(feature = "database")]
impl sqlx::Type<sqlx::Postgres> for UserRole {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <String as sqlx::Type<sqlx::Postgres>>::type_info()
    }
}

#[cfg(feature = "database")]
impl<'r> sqlx::Decode<'r, sqlx::Postgres> for UserRole {
    fn decode(value: sqlx::postgres::PgValueRef<'r>) -> Result<Self, sqlx::error::BoxDynError> {
        let s = <String as sqlx::Decode<sqlx::Postgres>>::decode(value)?;
        UserRole::try_from(s.as_str()).map_err(Into::into)
    }
}

#[cfg(feature = "database")]
impl sqlx::Encode<'_, sqlx::Postgres> for UserRole {
    fn encode_by_ref(
        &self,
        buf: &mut sqlx::postgres::PgArgumentBuffer,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        <&str as sqlx::Encode<sqlx::Postgres>>::encode_by_ref(&self.as_str(), buf)
    }
}

// =============================================================================
// RoutingMode
// =============================================================================

/// Trigger routing mode for message-to-task mapping.
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

#[cfg(feature = "database")]
impl sqlx::Type<sqlx::Postgres> for RoutingMode {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <String as sqlx::Type<sqlx::Postgres>>::type_info()
    }
}

#[cfg(feature = "database")]
impl<'r> sqlx::Decode<'r, sqlx::Postgres> for RoutingMode {
    fn decode(value: sqlx::postgres::PgValueRef<'r>) -> Result<Self, sqlx::error::BoxDynError> {
        let s = <String as sqlx::Decode<sqlx::Postgres>>::decode(value)?;
        RoutingMode::try_from(s.as_str()).map_err(Into::into)
    }
}

#[cfg(feature = "database")]
impl sqlx::Encode<'_, sqlx::Postgres> for RoutingMode {
    fn encode_by_ref(
        &self,
        buf: &mut sqlx::postgres::PgArgumentBuffer,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        <&str as sqlx::Encode<sqlx::Postgres>>::encode_by_ref(&self.as_str(), buf)
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

    pub fn is_replyable(&self) -> bool {
        true
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
            serde_json::to_string(&Scope::Team).expect("serialize"),
            "\"team\""
        );
        assert_eq!(
            serde_json::to_string(&Scope::User).expect("serialize"),
            "\"user\""
        );

        let parsed: Scope = serde_json::from_str("\"team\"").expect("deserialize");
        assert_eq!(parsed, Scope::Team);
    }

    #[test]
    fn scope_try_from() {
        assert_eq!(Scope::try_from("global"), Ok(Scope::Global));
        assert_eq!(Scope::try_from("team"), Ok(Scope::Team));
        assert_eq!(Scope::try_from("user"), Ok(Scope::User));
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
        assert!(AdapterKind::Linear.is_replyable());
    }
}
