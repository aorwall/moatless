/// No `Linear-Signature` header. Nothing is verified and nothing is admitted;
/// the header is absent rather than wrong.
pub const NO_SIGNATURE: &str = "no_signature";
/// The envelope names no organization, or names one with no
/// `adapter_connections` row.
pub use moatless_adapter_model::metrics::inbound_reason::NO_CONNECTION;
/// An `type` this Adapter does not handle.
pub const UNSUPPORTED_EVENT: &str = "unsupported_event";
/// A handled `type` with an `action` this Adapter does not act on — an Issue
/// neither created nor updated, a non-`create` Comment, a session neither
/// created nor prompted.
pub const UNSUPPORTED_ACTION: &str = "unsupported_action";
/// The inner payload is absent, does not parse, or parses without the field the
/// workflow keys on.
pub const MALFORMED_PAYLOAD: &str = "malformed_payload";
/// A session was created on a team with no active Subscription.
///
/// Closed with a terminal `error` activity so its Linear panel doesn't spin
/// forever.
pub const NO_SUBSCRIPTION: &str = "no_subscription";
/// A prompt carrying no body, or no activity to carry one.
pub const EMPTY_BODY: &str = "empty_body";
/// An `Issue`/`update` that neither moved the issue between workflow states nor
/// added a label — a title, a description, a removed label, an assignee. Linear
/// sends one for every field.
pub const NOT_A_STATE_CHANGE: &str = "not_a_state_change";
/// An `Issue`/`create` carrying no label, so no label trigger can fire.
pub const UNLABELED_CREATE: &str = "unlabeled_create";
