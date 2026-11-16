//! Trace the Workshop origin of untracked PBOs in addons/.
//!
//! The origin block maps a PBO to a cached Workshop folder. The identity
//! path (`--online`) searches the Workshop and prints a title only for a
//! confirmed match.

mod author;
mod bikey;
mod browse;
mod cluster;
mod evidence;
mod http;
mod identity;
// `match` is a keyword, so the module name needs a raw identifier; the
// file keeps the name the plan gives it.
mod r#match;
mod normalise;
mod online;
mod plan;
mod render;
mod resolve;
mod search;
mod store;
mod visibility;

pub use self::online::investigate;
