//! Message request/response types for UI consumption.
//!
//! Re-exported from the `moatless-api-schemas` crate.

pub use moatless_api_schemas::message::*;

use moatless_api_schemas::enums::{MessageRole as WireMessageRole, MessageType as WireMessageType};
use moatless_api_schemas::shared::AgentMode as WireAgentMode;
use moatless_core::shared_models::AgentMode;

use super::models::{MessageRole, MessageType};

pub fn agent_mode_from_wire(value: WireAgentMode) -> AgentMode {
    AgentMode(value.0)
}

pub fn agent_mode_to_wire(value: &AgentMode) -> WireAgentMode {
    WireAgentMode(value.0.clone())
}

pub fn message_type_to_wire(value: MessageType) -> WireMessageType {
    match value {
        MessageType::Assistant => WireMessageType::Assistant,
        MessageType::User => WireMessageType::User,
        MessageType::System => WireMessageType::System,
        MessageType::Result => WireMessageType::Result,
        MessageType::Error => WireMessageType::Error,
    }
}

pub fn message_type_from_wire(value: WireMessageType) -> MessageType {
    match value {
        WireMessageType::Assistant => MessageType::Assistant,
        WireMessageType::User => MessageType::User,
        WireMessageType::System => MessageType::System,
        WireMessageType::Result => MessageType::Result,
        WireMessageType::Error => MessageType::Error,
    }
}

pub fn message_role_to_wire(value: MessageRole) -> WireMessageRole {
    match value {
        MessageRole::User => WireMessageRole::User,
        MessageRole::Assistant => WireMessageRole::Assistant,
        MessageRole::System => WireMessageRole::System,
    }
}

pub fn message_role_from_wire(value: WireMessageRole) -> MessageRole {
    match value {
        WireMessageRole::User => MessageRole::User,
        WireMessageRole::Assistant => MessageRole::Assistant,
        WireMessageRole::System => MessageRole::System,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_message_discriminant_maps_both_ways_with_identical_json() {
        for domain in [
            MessageType::Assistant,
            MessageType::User,
            MessageType::System,
            MessageType::Result,
            MessageType::Error,
        ] {
            let wire = message_type_to_wire(domain);
            assert_eq!(message_type_from_wire(wire), domain);
            assert_eq!(
                serde_json::to_value(domain).expect("serialize"),
                serde_json::to_value(wire).expect("serialize")
            );
        }
        for domain in [
            MessageRole::User,
            MessageRole::Assistant,
            MessageRole::System,
        ] {
            let wire = message_role_to_wire(domain);
            assert_eq!(message_role_from_wire(wire), domain);
            assert_eq!(
                serde_json::to_value(domain).expect("serialize"),
                serde_json::to_value(wire).expect("serialize")
            );
        }
    }

    #[test]
    fn open_agent_mode_maps_both_ways_with_identical_json() {
        let domain = AgentMode("plan".to_string());
        let wire = agent_mode_to_wire(&domain);
        assert_eq!(agent_mode_from_wire(wire.clone()), domain);
        assert_eq!(
            serde_json::to_value(domain).expect("serialize"),
            serde_json::to_value(wire).expect("serialize")
        );
    }
}
