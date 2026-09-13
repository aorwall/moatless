/// One rule breach.
///
/// `key` identifies the breach across unrelated edits so a baseline entry does
/// not churn when a line moves; it never contains a line number — `locations`
/// carries those, and is printed but not baselined.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Violation {
    pub rule: &'static str,
    pub key: String,
    pub message: String,
    pub locations: Vec<String>,
}

impl Violation {
    /// A breach identified by its participants and located in the source tree.
    pub fn at(
        rule: &'static str,
        key: impl Into<String>,
        message: impl Into<String>,
        file: impl Into<String>,
    ) -> Self {
        Self {
            rule,
            key: key.into(),
            message: message.into(),
            locations: vec![file.into()],
        }
    }

    /// The stable identity a baseline entry records.
    pub fn id(&self) -> String {
        format!("{}::{}", self.rule, self.key)
    }
}
