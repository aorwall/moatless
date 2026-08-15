//! Canonical Loop vocabularies.

/// Kind of firing source owned by a Loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopKind {
    AdapterEvent,
    Schedule,
    Manual,
}

impl LoopKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AdapterEvent => "adapter_event",
            Self::Schedule => "schedule",
            Self::Manual => "manual",
        }
    }
}

impl TryFrom<&str> for LoopKind {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "adapter_event" => Ok(Self::AdapterEvent),
            "schedule" => Ok(Self::Schedule),
            "manual" => Ok(Self::Manual),
            other => Err(format!("unknown LoopKind: {other:?}")),
        }
    }
}

/// Authority from which the current Loop configuration was loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopConfigSource {
    Git,
    Manual,
}

impl LoopConfigSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Git => "git",
            Self::Manual => "manual",
        }
    }
}

impl TryFrom<&str> for LoopConfigSource {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "git" => Ok(Self::Git),
            "manual" => Ok(Self::Manual),
            other => Err(format!("unknown LoopConfigSource: {other:?}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_values_round_trip() {
        for kind in [LoopKind::AdapterEvent, LoopKind::Schedule, LoopKind::Manual] {
            assert_eq!(LoopKind::try_from(kind.as_str()), Ok(kind));
        }
        for source in [LoopConfigSource::Git, LoopConfigSource::Manual] {
            assert_eq!(LoopConfigSource::try_from(source.as_str()), Ok(source));
        }
    }
}
