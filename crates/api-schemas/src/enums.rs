//! Domain enums, re-exported from `moatless-types`.
//!
//! Every one of these is the domain enum itself. A DTO naming one names
//! the same type the DAO reads and the service decides on, so there is no
//! wire spelling to convert to.

pub use moatless_types::adapter::AdapterKind;
pub use moatless_types::task::CheckoutMode;
pub use moatless_types::message::MessageRole;
pub use moatless_types::message::MessageType;
pub use moatless_types::repository::RepositoryKind;
pub use moatless_types::repository::RepositoryProvider;
pub use moatless_types::loops::RoutingMode;
pub use moatless_types::sandbox::SandboxDesiredState;
pub use moatless_types::sandbox::SandboxStopReason;
pub use moatless_types::access_control::Scope;
pub use moatless_types::task::TaskRelationType;
pub use moatless_types::task::TaskStatus;
pub use moatless_types::user::UserRole;
pub use moatless_types::task::Visibility;
