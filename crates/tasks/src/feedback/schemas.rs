//! Feedback request/response types.
//!
//! Re-exported from the `moatless-api-schemas` crate.

pub use moatless_api_schemas::feedback::*;

use super::models::{
    FeedbackSource as DomainFeedbackSource, FeedbackSurface as DomainFeedbackSurface,
};

pub fn feedback_source_to_wire(value: DomainFeedbackSource) -> FeedbackSource {
    match value {
        DomainFeedbackSource::User => FeedbackSource::User,
        DomainFeedbackSource::Agent => FeedbackSource::Agent,
    }
}

pub fn feedback_source_from_wire(value: FeedbackSource) -> DomainFeedbackSource {
    match value {
        FeedbackSource::User => DomainFeedbackSource::User,
        FeedbackSource::Agent => DomainFeedbackSource::Agent,
    }
}

pub fn feedback_surface_from_wire(value: FeedbackSurface) -> DomainFeedbackSurface {
    match value {
        FeedbackSurface::Task => DomainFeedbackSurface::Task,
        FeedbackSurface::Message => DomainFeedbackSurface::Message,
        FeedbackSurface::ToolCall => DomainFeedbackSurface::ToolCall,
        FeedbackSurface::PreviewFailure => DomainFeedbackSurface::PreviewFailure,
    }
}

pub fn feedback_surface_to_wire(value: DomainFeedbackSurface) -> FeedbackSurface {
    match value {
        DomainFeedbackSurface::Task => FeedbackSurface::Task,
        DomainFeedbackSurface::Message => FeedbackSurface::Message,
        DomainFeedbackSurface::ToolCall => FeedbackSurface::ToolCall,
        DomainFeedbackSurface::PreviewFailure => FeedbackSurface::PreviewFailure,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_feedback_discriminant_maps_both_ways_with_identical_json() {
        for domain in [DomainFeedbackSource::User, DomainFeedbackSource::Agent] {
            let wire = feedback_source_to_wire(domain);
            assert_eq!(feedback_source_from_wire(wire), domain);
            assert_eq!(
                serde_json::to_value(domain).expect("serialize"),
                serde_json::to_value(wire).expect("serialize")
            );
        }
        for domain in [
            DomainFeedbackSurface::Task,
            DomainFeedbackSurface::Message,
            DomainFeedbackSurface::ToolCall,
            DomainFeedbackSurface::PreviewFailure,
        ] {
            let wire = feedback_surface_to_wire(domain);
            assert_eq!(feedback_surface_from_wire(wire), domain);
            assert_eq!(
                serde_json::to_value(domain).expect("serialize"),
                serde_json::to_value(wire).expect("serialize")
            );
        }
    }
}
