//! Contracts published by the sandbox domain.

pub use moatless_types::sandbox::{
    AgentStatus, LiveStatus, ProviderKind, SandboxError, SandboxStatus,
};
pub use moatless_types::sandbox::{ArchiveRetention, SandboxDesiredState, SandboxStopReason};
pub use moatless_types::task::{CheckoutMode, Task, TaskStatus};
pub use moatless_workspace_types::repository::Repository;

mod observation;
pub use observation::{
    ContractSandboxStatus, SandboxContainer, SandboxDetail, SandboxObserver, SandboxRuntimeEvent,
};

mod lifecycle;
pub use lifecycle::{
    SandboxIntentReader, SandboxLifecycle, SandboxOpError, SandboxOperator, SandboxStarter,
    SandboxStopper,
};

mod preflight;
#[cfg(feature = "test-support")]
pub use preflight::MockPreflightPort;
pub use preflight::{PreflightPort, PreflightRejection};

mod dispatch;
#[cfg(feature = "test-support")]
pub use dispatch::MockSandboxJoiner;
pub use dispatch::{SandboxJoiner, SandboxWaker};

mod snapshot;
#[cfg(feature = "test-support")]
pub use snapshot::MockWorkspaceSnapshotSource;
pub use snapshot::{
    SnapshotEntry, SnapshotError, SnapshotFile, SnapshotListing, WorkspaceSnapshotSource,
};

mod credentials;
pub use credentials::CodexCredentials;

mod port_publishing;
pub use port_publishing::SandboxPortPublisher;

mod diagnostics;
pub use diagnostics::{ClusterDiagnostics, SandboxDiagnosticsContainer, SandboxDiagnosticsEntry};
