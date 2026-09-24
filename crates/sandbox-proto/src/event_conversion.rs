use crate::EventType;

/// A domain event type the sandbox protocol has no representation for.
///
/// `sandbox.v1.EventType` is narrower than the domain type: `message.*` events
/// are derived by the backend while assembling `task_messages`, never
/// originating in a sandbox.
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
/// Partial by construction: the domain enum carries a value the protocol can't
/// express, and mapping it onto a neighboring variant would publish the wrong
/// type.
impl TryFrom<moatless_types::event::EventType> for EventType {
    type Error = UnrepresentableEventType;

    fn try_from(e: moatless_types::event::EventType) -> Result<Self, Self::Error> {
        match e {
            moatless_types::event::EventType::Task => Ok(EventType::Task),
            moatless_types::event::EventType::TaskTurn => Ok(EventType::TaskTurn),
            moatless_types::event::EventType::Sandbox => Ok(EventType::Sandbox),
            moatless_types::event::EventType::Agent => Ok(EventType::Agent),
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
            EventType::Resources => moatless_types::event::EventType::Resources,
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test fixtures must succeed"
)]
mod event_type_conversion_tests {
    use super::*;
    use moatless_types::event::EventType as Domain;

    /// Every domain event type, paired with whether the protocol can carry it.
    ///
    /// The `match` is exhaustive, so adding a domain variant fails to compile
    /// here until this wire contract is revisited — never silently mis-mapped.
    fn is_representable(e: Domain) -> bool {
        match e {
            Domain::Task
            | Domain::TaskTurn
            | Domain::Sandbox
            | Domain::Agent
            | Domain::Resources => true,
            Domain::Message => false,
        }
    }

    const ALL_DOMAIN: [Domain; 6] = [
        Domain::Task,
        Domain::TaskTurn,
        Domain::Sandbox,
        Domain::Agent,
        Domain::Resources,
        Domain::Message,
    ];

    const ALL_WIRE: [EventType; 5] = [
        EventType::Task,
        EventType::TaskTurn,
        EventType::Sandbox,
        EventType::Agent,
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
