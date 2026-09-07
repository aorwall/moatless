//! Stable reason labels for inbound Adapter routing outcomes.

/// No adapter connection matched the event's scope.
pub const NO_CONNECTION: &str = "no_connection";
/// No subscription matched the event's source.
pub const NO_RULE_MATCH: &str = "no_rule_match";
/// The matched subscription belongs to a different connection.
pub const WRONG_CONNECTION: &str = "wrong_connection";
/// The subscription is deactivated.
pub const INACTIVE_RULE: &str = "inactive_rule";
/// The binding's own task authored the event.
pub const SELF_ECHO: &str = "self_echo";
/// The binding already received this event.
pub const DUPLICATE_RECEIPT: &str = "duplicate_receipt";
/// Another delivery holds the creation claim for this subject, so this one
/// retries into the task that claim produces. Expected whenever a sender
/// emits one message as two events.
pub const CREATION_CLAIMED: &str = "creation_claimed";
/// The audience named someone else, so this binding holds nothing.
pub const NOT_IN_AUDIENCE: &str = "not_in_audience";
/// Every binding on a subject that had some was passed over, so the event
/// reached no task at all. The one outcome that is nobody's correct answer.
pub const NO_RECIPIENTS: &str = "no_recipients";
/// Delivered to a task the event itself created.
pub const NEW_TASK: &str = "new_task";
/// Delivered to a task that was already bound.
pub const EXISTING_TASK: &str = "existing_task";
/// Handing the message to the task message flow failed.
pub const DELIVER_FAILED: &str = "deliver_failed";
