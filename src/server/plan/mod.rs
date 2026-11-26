//! Compare the local pack with the remote workshop tree. Pure: every
//! filesystem fact arrives through [`PlanInput`].

mod compare;
mod model;

pub use compare::build_plan;
pub use model::{Action, Finding, Plan, PlanInput, RemoteFile, RemoteMod, Signature, Status};

/// The remote folder for one mod.
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
