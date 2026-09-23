//! Data structures shared across the investigate pipeline.

/// A Workshop search candidate with all ranking signals.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[allow(dead_code)] // fields stored for completeness; scored/displayed selectively
pub(crate) struct ScoredCandidate {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) search_score: f64,
    pub(crate) subscriptions: u64,
    pub(crate) views: u64,
    pub(crate) favorited: u64,
    pub(crate) star_rating: f64,
    pub(crate) total_votes: u32,
    pub(crate) tags: Vec<String>,
    pub(crate) creator: String,
    pub(crate) creator_name: String,
    pub(crate) time_updated: u64,
    pub(crate) short_description: String,
    pub(crate) children: Vec<String>,
    pub(crate) file_type: u32,
}

/// A group of PBOs that share the same search term.
/// Arma groups PBOs by `@ModName/` folder — all PBOs in one folder
/// load as one mod. Grouping by search term means we search once per
/// mod family instead of once per PBO.
pub(crate) struct PboGroup {
    pub(crate) search_term: String,
    /// Additional search terms derived from the full prefix path.
    /// E.g. for prefix "x\SPS\Vehicles\sps_blackhornet", the primary
    /// term is "SPS" but extras include "sps_blackhornet", "blackhornet".
    pub(crate) extra_terms: Vec<String>,
    pub(crate) pbos: Vec<String>,
    /// The full PBO prefix path (e.g. "z\ace\addons\grenades").
    /// Used for prefix path search in descriptions.
    pub(crate) prefix: Option<String>,
    pub(crate) author_handle: Option<String>,
    /// CfgPatches `author` field: full author name (e.g. "UnderSiege Productionz").
    pub(crate) cfg_author: Option<String>,
    /// CfgPatches `url` field: sometimes the exact Workshop page URL.
    pub(crate) cfg_url: Option<String>,
    /// CfgPatches class name: the addon's identity (e.g. "ffaa_data", "ade").
    /// Used as an additional search term.
    pub(crate) cfg_name: Option<String>,
    /// Required addons from CfgPatches: dependency mod families.
    pub(crate) cfg_children: Vec<String>,
}
