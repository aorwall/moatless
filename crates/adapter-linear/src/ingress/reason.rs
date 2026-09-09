/// No `Linear-Signature` header. Nothing is verified and nothing is
/// admitted; the header is absent rather than wrong.
pub const NO_SIGNATURE: &str = "no_signature";
/// The envelope names no organization, or names one with no
/// `adapter_connections` row.
pub const NO_CONNECTION: &str = "no_connection";
/// An `type` this Adapter does not handle.
pub const UNSUPPORTED_EVENT: &str = "unsupported_event";
/// A handled `type` with an `action` this Adapter does not act on — a
/// non-`update` Issue, a non-`create` Comment, a session neither created
/// nor prompted.
pub const UNSUPPORTED_ACTION: &str = "unsupported_action";
/// The inner payload is absent, does not parse, or parses without the field
/// the workflow keys on.
pub const MALFORMED_PAYLOAD: &str = "malformed_payload";
/// A session was created on a team with no active Subscription. The session
/// is closed with a terminal `error` activity so its Linear panel doesn't
/// spin forever; nothing durable records that Linear asked and we declined —
/// spec Open question 2, for which this reason is the interim answer.
pub const NO_SUBSCRIPTION: &str = "no_subscription";
/// A prompt carrying no body, or no activity to carry one.
pub const EMPTY_BODY: &str = "empty_body";
/// An `Issue`/`update` that did not move the issue between workflow states —
/// a title, a description, a label, an assignee. Linear sends one for every
/// field, and this Adapter acts on the state only.
pub const NOT_A_STATE_CHANGE: &str = "not_a_state_change";
