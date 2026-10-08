use moatless_types::agent::{AgentProfile, Harness};

/// The agent a [`Conversation`](crate::dao::Conversation) runs, as stored.
///
/// `Unknown` keeps an id this build does not know, such as one a newer backend
/// wrote during a rollout: the row still lists, and names no agent it is not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationAgent {
    Known(AgentProfile),
    Unknown(String),
}

impl ConversationAgent {
    /// The stored `agent_profile` id.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Known(profile) => profile.as_str(),
            Self::Unknown(id) => id,
        }
    }

    /// The driver family behind the agent; `None` when this build does not
    /// know the agent.
    pub fn harness(&self) -> Option<Harness> {
        match self {
            Self::Known(profile) => Some(profile.harness()),
            Self::Unknown(_) => None,
        }
    }
}

impl From<AgentProfile> for ConversationAgent {
    fn from(profile: AgentProfile) -> Self {
        Self::Known(profile)
    }
}

impl From<String> for ConversationAgent {
    fn from(id: String) -> Self {
        AgentProfile::parse(&id).map_or(Self::Unknown(id), Self::Known)
    }
}

impl std::fmt::Display for ConversationAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
