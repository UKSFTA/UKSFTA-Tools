//! Build a Keep a Changelog file from changes to the modpack composition.
//! The source is `mods.lock`, not git history: added, updated, and removed
//! Workshop items. Newest section first. An existing section for the same
//! version is replaced; every older section and the header are kept.

mod diff;
mod render;

#[cfg(test)]
mod tests;

pub use self::diff::{changes_for, next_state, snapshot, Changes};
pub use self::render::{can_replace, render_changelog};
