//! Prometheus metrics for the T3 UI-RPC surface.
//!
//! Emitted at the one seam every request crosses — the `ClientFrame::Request`
//! arm of `ProtocolSession::handle_json` — by reading the frames the handler
//! produced. Nothing else in this crate increments them.
//!
//! # Cardinality
//! The `tag` label is a method name the client chose, so it is bounded by
//! [`Outcome::tag_label`] rather than by trust: only an outcome that proves the
//! dispatch matched an arm carries the raw tag. See `crates/t3code/AGENTS.md`.

use serde_json::Value;

/// Completed UI-RPC requests by `tag` and `outcome`.
pub(crate) const REQUESTS: &str = "moatless_ui_rpc_requests_total";

/// Handler wall-clock in seconds by `tag` and `outcome`. Measures the frames a
/// request answered with, so a subscription's is its open, not its lifetime.
pub(crate) const REQUEST_DURATION: &str = "moatless_ui_rpc_request_duration_seconds";

/// Frames the socket refused before any dispatch, by `reason`.
pub(crate) const CONNECTION_DEFECTS: &str = "moatless_ui_rpc_connection_defects_total";

/// Register the descriptions. Called once per process, from [`crate::router`].
pub(crate) fn describe() {
    metrics::describe_counter!(REQUESTS, "T3 UI-RPC requests by method tag and outcome");
    metrics::describe_histogram!(
        REQUEST_DURATION,
        "T3 UI-RPC handler duration in seconds by method tag and outcome"
    );
    metrics::describe_counter!(
        CONNECTION_DEFECTS,
        "T3 UI-RPC frames refused before dispatch, by reason"
    );
}

/// What one request told the client, read off the frames it answered with.
///
/// The names are the client's vocabulary, not this crate's: `Fail` and `Die`
/// are the two `Cause` tags T3 decodes differently — a `Fail` is an outcome the
/// method's contract declares and the UI renders in its own words, a `Die` is a
/// defect the UI can only render as this server being broken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// `Exit` carrying `Success`.
    Success,
    /// A stream that opened and has not exited. The producer answers later on
    /// the socket, so this counts the subscribe and never its end.
    Open,
    /// The frame never reached the dispatch — a malformed or duplicate request
    /// id. Its `tag` is unverified and is not used as a label.
    Rejected,
    /// `Fail` carrying `UnsupportedMethodError`. Reachable for any tag at all,
    /// so its `tag` is not used as a label either.
    Unsupported,
    /// `Fail` carrying `EnvironmentAuthorizationError`: this deployment does not
    /// grant the scope the method needs.
    Unauthorized,
    /// Any other `Fail`.
    Fail,
    /// `Die`.
    Die,
    /// `Interrupt` — the client withdrew the request.
    Interrupted,
}

impl Outcome {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Open => "open",
            Self::Rejected => "rejected",
            Self::Unsupported => "unsupported",
            Self::Unauthorized => "unauthorized",
            Self::Fail => "fail",
            Self::Die => "die",
            Self::Interrupted => "interrupted",
        }
    }

    /// The `tag` label for a request that ended this way.
    ///
    /// [`Self::Rejected`] and [`Self::Unsupported`] are the two outcomes a tag
    /// no arm matched can reach, so they collapse to a constant. Every other
    /// outcome is produced by a matched arm, which makes the raw tag one of the
    /// enumerated method names and safe to label with.
    pub(crate) fn tag_label(self, tag: &str) -> &str {
        match self {
            Self::Rejected => "rejected",
            Self::Unsupported => "unsupported",
            _ => tag,
        }
    }

    /// Whether the client was told the request failed, for the span status and
    /// the log line. An [`Self::Interrupted`] request is not a failure: the
    /// client asked for it to stop.
    pub(crate) fn is_failure(self) -> bool {
        matches!(
            self,
            Self::Rejected | Self::Unsupported | Self::Unauthorized | Self::Fail | Self::Die
        )
    }
}

/// Read the outcome off the frames one request answered with.
///
/// A `Defect` frame outranks an `Exit` because it is what the pre-dispatch
/// checks emit, and those run before the tag is looked at.
pub(crate) fn classify(frames: &[Value]) -> Outcome {
    if frames.iter().any(|frame| frame["_tag"] == "Defect") {
        return Outcome::Rejected;
    }
    let Some(exit) = frames.iter().find(|frame| frame["_tag"] == "Exit") else {
        return Outcome::Open;
    };
    let exit = &exit["exit"];
    if exit["_tag"] == "Success" {
        return Outcome::Success;
    }
    let cause = &exit["cause"][0];
    match cause["_tag"].as_str() {
        Some("Interrupt") => Outcome::Interrupted,
        Some("Fail") => match cause["error"]["_tag"].as_str() {
            Some("UnsupportedMethodError") => Outcome::Unsupported,
            Some("EnvironmentAuthorizationError") => Outcome::Unauthorized,
            _ => Outcome::Fail,
        },
        // A `Die` and a cause shape this crate does not produce read the same
        // to the client: an unexpected server defect.
        _ => Outcome::Die,
    }
}

/// The defect string a [`Outcome::Die`] carried, for the log line.
pub(crate) fn defect(frames: &[Value]) -> Option<&str> {
    frames.iter().find_map(|frame| {
        frame["exit"]["cause"][0]["defect"]
            .as_str()
            .or_else(|| frame["defect"].as_str())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn exit(cause: Value) -> Vec<Value> {
        vec![json!({
            "_tag": "Exit",
            "requestId": "1",
            "exit": { "_tag": "Failure", "cause": [cause] }
        })]
    }

    #[test]
    fn a_success_exit_is_a_success() {
        let frames = vec![json!({
            "_tag": "Exit",
            "requestId": "1",
            "exit": { "_tag": "Success", "value": {} }
        })];
        assert_eq!(classify(&frames), Outcome::Success);
        assert!(!classify(&frames).is_failure());
    }

    #[test]
    fn a_stream_that_only_opened_has_not_exited() {
        let frames = vec![json!({ "_tag": "Chunk", "requestId": "1", "values": [] })];
        assert_eq!(classify(&frames), Outcome::Open);
    }

    #[test]
    fn a_chunk_before_an_exit_does_not_hide_the_exit() {
        let mut frames = vec![json!({ "_tag": "Chunk", "requestId": "1", "values": [] })];
        frames.extend(exit(json!({ "_tag": "Die", "defect": "boom" })));
        assert_eq!(classify(&frames), Outcome::Die);
    }

    /// The two tagged errors that get their own outcome, because an operator
    /// reads them differently: one is a client asking for a method this server
    /// does not have, the other is this deployment refusing a scope.
    #[test]
    fn the_two_named_tagged_errors_split_out_of_fail() {
        assert_eq!(
            classify(&exit(
                json!({ "_tag": "Fail", "error": { "_tag": "UnsupportedMethodError" } })
            )),
            Outcome::Unsupported
        );
        assert_eq!(
            classify(&exit(
                json!({ "_tag": "Fail", "error": { "_tag": "EnvironmentAuthorizationError" } })
            )),
            Outcome::Unauthorized
        );
        assert_eq!(
            classify(&exit(
                json!({ "_tag": "Fail", "error": { "_tag": "ProjectReadFileError" } })
            )),
            Outcome::Fail
        );
    }

    #[test]
    fn an_interrupt_is_not_a_failure() {
        let outcome = classify(&exit(json!({ "_tag": "Interrupt" })));
        assert_eq!(outcome, Outcome::Interrupted);
        assert!(!outcome.is_failure());
    }

    /// A `Defect` frame answers the socket, not the request, so it is what the
    /// pre-dispatch id checks emit — and the tag beside it was never matched.
    #[test]
    fn a_defect_frame_is_rejected_and_does_not_label_with_its_tag() {
        let frames = vec![json!({ "_tag": "Defect", "defect": "Duplicate id" })];
        let outcome = classify(&frames);
        assert_eq!(outcome, Outcome::Rejected);
        assert_eq!(
            outcome.tag_label("anything.the.client.invented"),
            "rejected"
        );
        assert_eq!(defect(&frames), Some("Duplicate id"));
    }

    #[test]
    fn an_unsupported_method_does_not_label_with_its_tag() {
        assert_eq!(
            Outcome::Unsupported.tag_label("nonsense.method"),
            "unsupported"
        );
        assert_eq!(
            Outcome::Fail.tag_label("projects.readFile"),
            "projects.readFile"
        );
    }
}
