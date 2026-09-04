//! Generated gRPC types and services for sandbox <-> backend communication.

pub mod client;

pub mod sandbox {
    pub mod v1 {
        // Generated protobuf message enums carry size-disparate variants by
        // design (a bare bool beside a full result message); boxing them to
        // placate the lint would only churn every construction site.
        #![allow(clippy::large_enum_variant)]
        tonic::include_proto!("sandbox.v1");
    }
}

// Re-export for convenience
pub use sandbox::v1::*;

/// Whether `path` is a workspace-relative path a
/// [`GetWorkspaceObjectUrlsRequest`] may carry.
///
/// The producer (the sandbox) and the validator (the backend) share this one
/// rule so a path the sandbox sends is never one the backend refuses — a
/// rejected batch would wedge every later mirror sync. S3 keys are opaque, so
/// `..` does not traverse there; the guard exists for the two ends that do
/// resolve it — a restore writes each path under `WORKSPACE_DIR`, and a reader
/// serving a stopped task's files resolves it again. Rejects an absolute path,
/// any `.`/`..` component, an empty path, and one long enough to overrun S3's
/// 1024-byte key limit once the prefix is on it.
pub fn is_safe_workspace_path(path: &str) -> bool {
    if path.is_empty() || path.len() > 900 || path.starts_with('/') {
        return false;
    }
    path.split('/')
        .all(|part| !part.is_empty() && part != "." && part != "..")
}

// Re-export the domain event types for consumers
pub use moatless_types::event;

// =============================================================================
// Conversions: moatless_types::event::EventType <-> proto EventType
//
// These live here because `sandbox.v1.EventType` and `moatless_types::event::EventType`
// are both foreign to every consumer crate, so the orphan rule puts the trait
// impls in the crate that owns the wire type. Nothing else is hoisted here:
// adapter DTO conversions stay beside their own domain type.
// =============================================================================

/// A domain event type the sandbox protocol has no representation for.
///
/// `sandbox.v1.EventType` is deliberately narrower than
/// `moatless_types::event::EventType`: `message.*` events are derived by the backend
/// while assembling `task_messages` (the agent.message skeleton) and never originate in a sandbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnrepresentableEventType(pub moatless_types::event::EventType);

impl std::fmt::Display for UnrepresentableEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "event type `{}` has no sandbox protocol representation",
            self.0
        )
    }
}

impl std::error::Error for UnrepresentableEventType {}

/// Narrows a domain event type onto the wire enum.
///
/// Partial by construction — the domain enum carries a value the protocol
/// cannot express, and mapping it onto a neighbouring variant would publish an
/// event under the wrong type.
impl TryFrom<moatless_types::event::EventType> for EventType {
    type Error = UnrepresentableEventType;

    fn try_from(e: moatless_types::event::EventType) -> Result<Self, Self::Error> {
        match e {
            moatless_types::event::EventType::Task => Ok(EventType::Task),
            moatless_types::event::EventType::TaskTurn => Ok(EventType::TaskTurn),
            moatless_types::event::EventType::Sandbox => Ok(EventType::Sandbox),
            moatless_types::event::EventType::Agent => Ok(EventType::Agent),
            moatless_types::event::EventType::Server => Ok(EventType::Server),
            moatless_types::event::EventType::Resources => Ok(EventType::Resources),
            // Derived, UI-only event. Rejected rather than mapped: publishing it
            // as any other type would silently corrupt the event stream.
            moatless_types::event::EventType::Message => Err(UnrepresentableEventType(e)),
        }
    }
}

/// Every wire variant has exactly one domain counterpart, so widening is
/// lossless and infallible. Unknown integers are rejected before this point by
/// prost's generated `TryFrom<i32>`.
impl From<EventType> for moatless_types::event::EventType {
    fn from(e: EventType) -> Self {
        match e {
            EventType::Task => moatless_types::event::EventType::Task,
            EventType::TaskTurn => moatless_types::event::EventType::TaskTurn,
            EventType::Sandbox => moatless_types::event::EventType::Sandbox,
            EventType::Agent => moatless_types::event::EventType::Agent,
            EventType::Server => moatless_types::event::EventType::Server,
            EventType::Resources => moatless_types::event::EventType::Resources,
        }
    }
}

#[cfg(test)]
mod event_type_conversion_tests {
    use super::*;
    use moatless_types::event::EventType as Domain;

    /// Every domain event type, paired with whether the protocol can carry it.
    ///
    /// The `match` is exhaustive, so adding a variant to
    /// `moatless_types::event::EventType` fails to compile until this wire contract is
    /// revisited — it cannot be silently mis-mapped.
    fn is_representable(e: Domain) -> bool {
        match e {
            Domain::Task
            | Domain::TaskTurn
            | Domain::Sandbox
            | Domain::Agent
            | Domain::Server
            | Domain::Resources => true,
            Domain::Message => false,
        }
    }

    const ALL_DOMAIN: [Domain; 7] = [
        Domain::Task,
        Domain::TaskTurn,
        Domain::Sandbox,
        Domain::Agent,
        Domain::Server,
        Domain::Resources,
        Domain::Message,
    ];

    const ALL_WIRE: [EventType; 6] = [
        EventType::Task,
        EventType::TaskTurn,
        EventType::Sandbox,
        EventType::Agent,
        EventType::Server,
        EventType::Resources,
    ];

    #[test]
    fn every_representable_domain_type_round_trips_exactly() {
        for domain in ALL_DOMAIN {
            let wire = EventType::try_from(domain);
            if is_representable(domain) {
                let wire = wire.expect("representable type converts");
                assert_eq!(Domain::from(wire), domain, "round trip changed {domain}");
            } else {
                assert_eq!(wire, Err(UnrepresentableEventType(domain)));
            }
        }
    }

    #[test]
    fn every_wire_type_round_trips_exactly() {
        for wire in ALL_WIRE {
            let domain = Domain::from(wire);
            assert_eq!(
                EventType::try_from(domain),
                Ok(wire),
                "round trip changed {wire:?}"
            );
            // The discriminant is the wire contract; it must survive i32 too.
            assert_eq!(EventType::try_from(wire as i32), Ok(wire));
        }
    }

    #[test]
    fn domain_list_covers_every_variant() {
        assert_eq!(
            ALL_DOMAIN.len(),
            ALL_WIRE.len() + 1,
            "exactly one domain variant (`message`) is unrepresentable"
        );
        assert_eq!(
            ALL_DOMAIN
                .iter()
                .copied()
                .filter(|e| is_representable(*e))
                .count(),
            ALL_WIRE.len()
        );
    }

    #[test]
    fn message_is_rejected_with_a_contextual_error() {
        let err = EventType::try_from(Domain::Message).expect_err("message is UI-only");
        assert_eq!(err.0, Domain::Message);
        assert_eq!(
            err.to_string(),
            "event type `message` has no sandbox protocol representation"
        );
    }

    #[test]
    fn unknown_wire_integers_are_rejected() {
        for unknown in [-1, 6, 7, i32::MAX] {
            assert!(
                EventType::try_from(unknown).is_err(),
                "{unknown} must not decode to an event type"
            );
        }
    }
}
