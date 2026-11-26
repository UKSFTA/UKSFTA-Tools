//! Compare the local build with the remote pack folder. Pure: every
//! filesystem fact arrives through [`PlanInput`].

mod compare;
mod model;
mod tree;

pub use compare::build_plan;
pub use model::{
    is_addons_pbo, is_addons_signature, is_key_file, is_pbo, Action, Finding, LocalPack, Plan,
    PlanInput, RemoteEntry, RemotePack,
};
pub use tree::{inside, walk_release, LocalEntry};

/// The remote folder for one pack id: `<root>/@<id>`.
pub fn mod_dir(root: &str, id: &str) -> String {
    join_remote(root, &format!("@{id}"))
}

/// Join a POSIX remote path. The root may be `/`.
pub fn join_remote(root: &str, relative: &str) -> String {
    let base = root.trim_end_matches('/');
    if relative.is_empty() {
        if base.is_empty() {
            "/".to_string()
        } else {
            base.to_string()
        }
    } else {
        format!("{base}/{relative}")
    }
}

#[cfg(test)]
mod tests;
