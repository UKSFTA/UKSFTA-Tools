//! PBO grouping by mod family.

use std::collections::HashMap;
use std::path::Path;

use super::model::PboGroup;
use crate::origin::ResolvedOrigin;
use crate::pbo::{
    extra_search_terms_from_prefix, pbo_cfg_patches, pbo_config_identity, pbo_prefix,
    search_term_from_prefix,
};
use crate::prefix::prefix_family;

/// Group PBOs by their mod family. The grouping key is the prefix
/// family (first 2 segments for namespaced roots, full prefix otherwise,
/// filename stem for bare names). Config identity is a scoring signal
/// only — it must not determine grouping because PBOs from the same mod
/// can have different config identities (e.g. `requiredAddons` varies
/// per PBO).
pub(crate) fn group_pbos_by_term(
    pbos: &[(String, Option<ResolvedOrigin>)],
    addons_dir: &Path,
) -> Vec<PboGroup> {
    let mut groups: HashMap<String, PboGroup> = HashMap::new();

    for (name, _origin) in pbos {
        let pbo_path = addons_dir.join(name);

        // Primary grouping key: prefix family
        let prefix = pbo_prefix(&pbo_path);
        let group_key = match prefix {
            Some(ref p) => prefix_family(p),
            None => {
                // No prefix — use filename stem before first `_`,
                // stripping the .pbo extension first
                let stem = name.strip_suffix(".pbo").unwrap_or(name);
                stem.split('_').next().unwrap_or(stem).to_string()
            }
        };

        // Search term for the Workshop query: derive from the group key.
        let search_term = search_term_from_prefix(&group_key);

        // Author handle from config identity (short handle, scoring signal only)
        let author_handle = pbo_config_identity(&pbo_path).filter(|t| t.len() < 6 && t.len() >= 2);

        // Full CfgPatches data: author, url, required addons.
        // Read from the first PBO in each group (free cross-reference signals).
        let cfg = pbo_cfg_patches(&pbo_path);

        let group = groups.entry(group_key.clone()).or_insert_with(|| {
            let full_prefix = prefix.clone().unwrap_or_default();
            let extra_terms = extra_search_terms_from_prefix(&full_prefix);
            PboGroup {
                search_term,
                extra_terms,
                pbos: Vec::new(),
                prefix: prefix.clone(),
                author_handle: None,
                cfg_author: cfg.author.clone(),
                cfg_url: cfg.url.clone(),
                cfg_name: cfg.name.clone(),
                cfg_children: cfg.required_addons.clone(),
            }
        });
        group.pbos.push(name.clone());
        if let Some(ref ah) = author_handle {
            if group
                .author_handle
                .as_ref()
                .is_none_or(|existing| ah.len() > existing.len())
            {
                group.author_handle = Some(ah.clone());
            }
        }
        // If the first PBO had no author/url/name, try this one
        if group.cfg_author.is_none() && cfg.author.is_some() {
            group.cfg_author = cfg.author;
        }
        if group.cfg_url.is_none() && cfg.url.is_some() {
            group.cfg_url = cfg.url;
        }
        if group.cfg_name.is_none() && cfg.name.is_some() {
            group.cfg_name = cfg.name;
        }
    }

    groups.into_values().collect()
}
