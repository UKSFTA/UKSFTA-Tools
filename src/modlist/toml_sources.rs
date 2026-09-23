use serde::{Deserialize, Serialize};

/// TOML v2 schema for mod_sources.txt
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct TomlModSources {
    #[serde(default = "default_sources_version")]
    pub(crate) version: u32,
    #[serde(default)]
    pub(crate) mods: Vec<TomlMod>,
}

pub(crate) fn default_sources_version() -> u32 {
    2
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct TomlMod {
    pub(crate) id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) tags: Vec<String>,
    #[serde(default = "default_role", skip_serializing_if = "is_default_role")]
    pub(crate) role: String,
    #[serde(
        default = "default_enabled",
        skip_serializing_if = "is_default_enabled"
    )]
    pub(crate) enabled: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) dependencies: Vec<String>,
}

pub(crate) fn default_role() -> String {
    "mod".to_string()
}

pub(crate) fn default_enabled() -> bool {
    true
}

pub(crate) fn is_default_role(role: &str) -> bool {
    role == "mod"
}

pub(crate) fn is_default_enabled(enabled: &bool) -> bool {
    *enabled
}
