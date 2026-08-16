//! Task request/response types.
//!
//! Re-exported from the `moatless-api-schemas` crate.

pub use moatless_api_schemas::task::*;

use moatless_adapter_model::models::AdapterKind as DomainAdapterKind;
use moatless_api_schemas::enums::{
    AdapterKind as WireAdapterKind, CheckoutMode as WireCheckoutMode,
    SandboxDesiredState as WireSandboxDesiredState, SandboxStopReason as WireSandboxStopReason,
    Scope as WireScope, TaskRelationType as WireTaskRelationType, TaskStatus as WireTaskStatus,
    Visibility as WireVisibility,
};
use moatless_api_schemas::shared::AgentType as WireAgentType;
use moatless_backend_core::scope::Scope;
use moatless_core::shared_models::AgentType;

use super::models::{CheckoutMode, SandboxDesiredState, SandboxStopReason, TaskStatus, Visibility};
use super::relations::TaskRelationType;

pub fn agent_type_from_wire(value: WireAgentType) -> AgentType {
    AgentType(value.0)
}

pub fn agent_type_to_wire(value: &AgentType) -> WireAgentType {
    WireAgentType(value.0.clone())
}

pub fn checkout_mode_from_wire(value: WireCheckoutMode) -> CheckoutMode {
    match value {
        WireCheckoutMode::Shared => CheckoutMode::Shared,
        WireCheckoutMode::Isolated => CheckoutMode::Isolated,
    }
}

pub fn checkout_mode_to_wire(value: CheckoutMode) -> WireCheckoutMode {
    match value {
        CheckoutMode::Shared => WireCheckoutMode::Shared,
        CheckoutMode::Isolated => WireCheckoutMode::Isolated,
    }
}

pub fn task_status_from_wire(value: WireTaskStatus) -> TaskStatus {
    match value {
        WireTaskStatus::Open => TaskStatus::Open,
        WireTaskStatus::Closed => TaskStatus::Closed,
        WireTaskStatus::Error => TaskStatus::Error,
    }
}

pub fn task_status_to_wire(value: TaskStatus) -> WireTaskStatus {
    match value {
        TaskStatus::Open => WireTaskStatus::Open,
        TaskStatus::Closed => WireTaskStatus::Closed,
        TaskStatus::Error => WireTaskStatus::Error,
    }
}

pub fn sandbox_desired_state_to_wire(value: SandboxDesiredState) -> WireSandboxDesiredState {
    match value {
        SandboxDesiredState::Running => WireSandboxDesiredState::Running,
        SandboxDesiredState::Stopped => WireSandboxDesiredState::Stopped,
        SandboxDesiredState::Removed => WireSandboxDesiredState::Removed,
    }
}

pub fn sandbox_desired_state_from_wire(value: WireSandboxDesiredState) -> SandboxDesiredState {
    match value {
        WireSandboxDesiredState::Running => SandboxDesiredState::Running,
        WireSandboxDesiredState::Stopped => SandboxDesiredState::Stopped,
        WireSandboxDesiredState::Removed => SandboxDesiredState::Removed,
    }
}

pub fn sandbox_stop_reason_to_wire(value: SandboxStopReason) -> WireSandboxStopReason {
    match value {
        SandboxStopReason::User => WireSandboxStopReason::User,
        SandboxStopReason::IdleReaper => WireSandboxStopReason::IdleReaper,
        SandboxStopReason::Cleanup => WireSandboxStopReason::Cleanup,
        SandboxStopReason::System => WireSandboxStopReason::System,
        SandboxStopReason::Error => WireSandboxStopReason::Error,
    }
}

pub fn sandbox_stop_reason_from_wire(value: WireSandboxStopReason) -> SandboxStopReason {
    match value {
        WireSandboxStopReason::User => SandboxStopReason::User,
        WireSandboxStopReason::IdleReaper => SandboxStopReason::IdleReaper,
        WireSandboxStopReason::Cleanup => SandboxStopReason::Cleanup,
        WireSandboxStopReason::System => SandboxStopReason::System,
        WireSandboxStopReason::Error => SandboxStopReason::Error,
    }
}

pub fn relation_type_to_wire(value: TaskRelationType) -> WireTaskRelationType {
    match value {
        TaskRelationType::CreatedBy => WireTaskRelationType::CreatedBy,
        TaskRelationType::RelatedTo => WireTaskRelationType::RelatedTo,
    }
}

pub fn relation_type_from_wire(value: WireTaskRelationType) -> TaskRelationType {
    match value {
        WireTaskRelationType::CreatedBy => TaskRelationType::CreatedBy,
        WireTaskRelationType::RelatedTo => TaskRelationType::RelatedTo,
    }
}

pub fn visibility_from_wire(value: WireVisibility) -> Visibility {
    match value {
        WireVisibility::Public => Visibility::Public,
        WireVisibility::Private => Visibility::Private,
    }
}

pub fn visibility_to_wire(value: Visibility) -> WireVisibility {
    match value {
        Visibility::Public => WireVisibility::Public,
        Visibility::Private => WireVisibility::Private,
    }
}

pub fn scope_to_wire(value: Scope) -> WireScope {
    match value {
        Scope::Global => WireScope::Global,
        Scope::User => WireScope::User,
    }
}

pub fn scope_from_wire(value: WireScope) -> Scope {
    match value {
        WireScope::Global => Scope::Global,
        WireScope::User => Scope::User,
    }
}

pub fn adapter_kind_to_wire(value: DomainAdapterKind) -> WireAdapterKind {
    match value {
        DomainAdapterKind::Slack => WireAdapterKind::Slack,
        DomainAdapterKind::Telegram => WireAdapterKind::Telegram,
        DomainAdapterKind::Github => WireAdapterKind::Github,
        DomainAdapterKind::GithubPr => WireAdapterKind::GithubPr,
        DomainAdapterKind::Linear => WireAdapterKind::Linear,
        DomainAdapterKind::Webhook => WireAdapterKind::Webhook,
    }
}

pub fn adapter_kind_from_wire(value: WireAdapterKind) -> DomainAdapterKind {
    match value {
        WireAdapterKind::Slack => DomainAdapterKind::Slack,
        WireAdapterKind::Telegram => DomainAdapterKind::Telegram,
        WireAdapterKind::Github => DomainAdapterKind::Github,
        WireAdapterKind::GithubPr => DomainAdapterKind::GithubPr,
        WireAdapterKind::Linear => DomainAdapterKind::Linear,
        WireAdapterKind::Webhook => DomainAdapterKind::Webhook,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn same_json<T: serde::Serialize, U: serde::Serialize>(domain: T, wire: U) {
        assert_eq!(
            serde_json::to_value(domain).expect("serialize domain"),
            serde_json::to_value(wire).expect("serialize wire")
        );
    }

    #[test]
    fn every_task_discriminant_maps_both_ways_with_identical_json() {
        for domain in [CheckoutMode::Shared, CheckoutMode::Isolated] {
            let wire = checkout_mode_to_wire(domain);
            assert_eq!(checkout_mode_from_wire(wire), domain);
            same_json(domain, wire);
        }
        for domain in [TaskStatus::Open, TaskStatus::Closed, TaskStatus::Error] {
            let wire = task_status_to_wire(domain);
            assert_eq!(task_status_from_wire(wire), domain);
            same_json(domain, wire);
        }
        for domain in [
            SandboxDesiredState::Running,
            SandboxDesiredState::Stopped,
            SandboxDesiredState::Removed,
        ] {
            let wire = sandbox_desired_state_to_wire(domain);
            assert_eq!(sandbox_desired_state_from_wire(wire), domain);
            same_json(domain, wire);
        }
        for domain in [
            SandboxStopReason::User,
            SandboxStopReason::IdleReaper,
            SandboxStopReason::Cleanup,
            SandboxStopReason::System,
            SandboxStopReason::Error,
        ] {
            let wire = sandbox_stop_reason_to_wire(domain);
            assert_eq!(sandbox_stop_reason_from_wire(wire), domain);
            same_json(domain, wire);
        }
        for domain in [TaskRelationType::CreatedBy, TaskRelationType::RelatedTo] {
            let wire = relation_type_to_wire(domain);
            assert_eq!(relation_type_from_wire(wire), domain);
            same_json(domain, wire);
        }
        for domain in [Visibility::Public, Visibility::Private] {
            let wire = visibility_to_wire(domain);
            assert_eq!(visibility_from_wire(wire), domain);
            same_json(domain, wire);
        }
        for domain in [Scope::Global, Scope::User] {
            let wire = scope_to_wire(domain);
            assert_eq!(scope_from_wire(wire), domain);
            same_json(domain, wire);
        }
        for domain in [
            DomainAdapterKind::Slack,
            DomainAdapterKind::Telegram,
            DomainAdapterKind::Github,
            DomainAdapterKind::GithubPr,
            DomainAdapterKind::Linear,
            DomainAdapterKind::Webhook,
        ] {
            let wire = adapter_kind_to_wire(domain);
            assert_eq!(adapter_kind_from_wire(wire), domain);
            same_json(domain, wire);
        }
    }

    #[test]
    fn open_agent_identifier_maps_both_ways_with_identical_json() {
        let domain = AgentType("custom-agent".to_string());
        let wire = agent_type_to_wire(&domain);
        assert_eq!(agent_type_from_wire(wire.clone()), domain);
        same_json(domain, wire);
    }
}
