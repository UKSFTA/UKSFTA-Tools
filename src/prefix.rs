//! Shared backslash and underscore tokenisation for PBO prefixes.
//!
//! A PBO prefix is the addon's canonical virtual path, for example
//! `z\ace\addons\grenades`. The prefix root, the prefix family, and the
//! Workshop search term all split that path into components and identity
//! tokens. One copy of the split rules stops the three callers from
//! drifting apart.

/// Split a prefix into its backslash-separated path components.
pub(crate) fn split_backslash(prefix: &str) -> Vec<&str> {
    prefix.split('\\').collect()
}

/// The root of an addon prefix: the first path component before a
/// backslash (e.g. "z\ace\addons\grenades" -> "z"). A standalone mod's
/// PBOs share few roots; an aggregate pack spans many.
pub(crate) fn prefix_root(prefix: &str) -> &str {
    split_backslash(prefix).first().copied().unwrap_or(prefix)
}

/// Extract a grouping key from a PBO prefix. This identifies the mod
/// family and is used to group PBOs that belong to the same Workshop
/// item.
///
/// Rules (by prefix structure):
/// - Namespaced root (`z\`, `x\`, `pz\`, `v\`, `a3\`): first 2 segments
///   (`z\aceax\addons\main` -> `z\aceax`). This keeps mod families
///   together while preventing `z\` from merging all third-party mods.
/// - Non-namespaced root (`NAVSPECWARGRU2\common`): full prefix. The
///   root IS the mod identity.
pub(crate) fn prefix_family(prefix: &str) -> String {
    let parts = split_backslash(prefix);
    if parts.len() >= 2 && parts[0].len() < 3 {
        format!("{}\\{}", parts[0], parts[1])
    } else {
        prefix.to_string()
    }
}

/// The first underscore-separated token, or the whole string when there
/// is no underscore.
pub(crate) fn first_underscore_token(s: &str) -> &str {
    s.split('_').next().unwrap_or(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_root_takes_first_backslash_component() {
        assert_eq!(prefix_root("z\\ace\\addons\\grenades"), "z");
        assert_eq!(prefix_root("TFL_Headgear"), "TFL_Headgear");
        assert_eq!(prefix_root(""), "");
    }

    #[test]
    fn prefix_family_keeps_namespace_pair_only_for_short_root() {
        assert_eq!(prefix_family("z\\aceax\\addons\\main"), "z\\aceax");
        assert_eq!(
            prefix_family("NAVSPECWARGRU2\\common"),
            "NAVSPECWARGRU2\\common"
        );
        assert_eq!(
            prefix_family("aceax_compat_tfl_cold"),
            "aceax_compat_tfl_cold"
        );
    }

    #[test]
    fn first_underscore_token_splits_once() {
        assert_eq!(first_underscore_token("TFL_Headgear"), "TFL");
        assert_eq!(first_underscore_token("TFL"), "TFL");
        assert_eq!(first_underscore_token(""), "");
    }

    #[test]
    fn search_term_from_prefix_uses_distinctive_token() {
        assert_eq!(crate::pbo::search_term_from_prefix("TFL_Headgear"), "TFL");
        assert_eq!(
            crate::pbo::search_term_from_prefix("NAVSPECWARGRU2_TACDEV"),
            "NAVSPECWARGRU2"
        );
        assert_eq!(
            crate::pbo::search_term_from_prefix("z\\ace\\addons\\grenades"),
            "ace"
        );
        assert_eq!(crate::pbo::search_term_from_prefix("z"), "z");
        assert_eq!(
            crate::pbo::search_term_from_prefix("x\\zen\\addons\\ai"),
            "zen"
        );
    }
}
