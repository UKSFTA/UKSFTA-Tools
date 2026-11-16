//! Shared backslash tokenisation for PBO prefixes.
//!
//! A PBO prefix is the addon's canonical virtual path, for example
//! `z\ace\addons\grenades`. The prefix root splits that path into
//! components. One copy of the split rule stops callers drifting apart.

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_root_takes_first_backslash_component() {
        assert_eq!(prefix_root("z\\ace\\addons\\grenades"), "z");
        assert_eq!(prefix_root("TFL_Headgear"), "TFL_Headgear");
        assert_eq!(prefix_root(""), "");
    }
}
