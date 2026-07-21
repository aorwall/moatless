//! Generated gRPC types and services for sandbox <-> backend communication.

pub mod sandbox {
    pub mod v1 {
        tonic::include_proto!("sandbox.v1");
    }
}

// Re-export for convenience
pub use sandbox::v1::*;

// Re-export moatless_events for consumers
pub use moatless_events;

// =============================================================================
// Conversions: moatless_events::EventType <-> proto EventType
// =============================================================================

impl From<moatless_events::EventType> for EventType {
    fn from(e: moatless_events::EventType) -> Self {
        match e {
            moatless_events::EventType::Task => EventType::Task,
            moatless_events::EventType::TaskTurn => EventType::TaskTurn,
            moatless_events::EventType::Sandbox => EventType::Sandbox,
            moatless_events::EventType::Agent => EventType::Agent,
            moatless_events::EventType::Server => EventType::Server,
            moatless_events::EventType::Resources => EventType::Resources,
            // Derived, UI-only event; never sent over the sandbox proto. This
            // arm exists only for exhaustiveness — the sandbox has no `Message`.
            moatless_events::EventType::Message => EventType::Resources,
        }
    }
}

impl TryFrom<EventType> for moatless_events::EventType {
    type Error = &'static str;

    fn try_from(e: EventType) -> Result<Self, Self::Error> {
        match e {
            EventType::Task => Ok(moatless_events::EventType::Task),
            EventType::TaskTurn => Ok(moatless_events::EventType::TaskTurn),
            EventType::Sandbox => Ok(moatless_events::EventType::Sandbox),
            EventType::Agent => Ok(moatless_events::EventType::Agent),
            EventType::Server => Ok(moatless_events::EventType::Server),
            EventType::Resources => Ok(moatless_events::EventType::Resources),
        }
    }
}
