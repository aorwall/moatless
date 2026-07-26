//! Skill domain types.

pub use moatless_api_schemas::skill::SkillSource;

/// Internal representation of a parsed skill before it's assembled into a
/// `SkillResponse`. Kept separate so the scanner can emit source-less rows.
#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub source: SkillSource,
    pub plugin_id: Option<String>,
    pub plugin_name: Option<String>,
}
