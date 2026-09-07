//! Contracts published by the task domain.

pub use moatless_types::task::{TaskStatus, TurnStatus};

mod reading;
#[cfg(feature = "test-support")]
pub use reading::MockTaskReader;
pub use reading::{TaskDescriptor, TaskReader};

mod transcript;
#[cfg(feature = "test-support")]
pub use transcript::MockTranscriptAppend;
pub use transcript::{
    AssistantReply, EventSubject, ExternalEvent, ToolCallRecord, TranscriptAppend, TranscriptEntry,
    TranscriptEntryKind, TranscriptOrigin,
};

mod turn;
#[cfg(feature = "test-support")]
pub use turn::{MockTurnCounter, MockTurnStateReader};
pub use turn::{TurnCounter, TurnSignals, TurnStateReader};

mod tags;
#[cfg(feature = "test-support")]
pub use tags::MockTagResolver;
pub use tags::TagResolver;

mod opening;
#[cfg(feature = "test-support")]
pub use opening::MockTaskOpener;
pub use opening::{NewTask, SandboxProvision, TaskOpenError, TaskOpened, TaskOpener};

mod message;
#[cfg(feature = "test-support")]
pub use message::MockMessageAcceptor;
pub use message::{InboundTurn, MessageAcceptor, TurnOrigin};

mod closing;
#[cfg(feature = "test-support")]
pub use closing::MockTaskFinisher;
pub use closing::{CloseReason, TaskFinisher};

mod relations;
#[cfg(feature = "test-support")]
pub use relations::MockRelationRecorder;
pub use relations::RelationRecorder;

mod pending_work;
#[cfg(feature = "test-support")]
pub use pending_work::MockPendingWorkReader;
pub use pending_work::PendingWorkReader;
