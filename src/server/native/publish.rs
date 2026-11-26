//! Publish the verified temp file with a rename.

use openssh_sftp_client::metadata::MetaDataBuilder;
use openssh_sftp_client::{Sftp, UnixTimeStamp};

use super::writer::plan_aside_name;
use crate::error::UksftaError;

/// How the temp file reached the final name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishMethod {
    PosixRename,
    RenameAside,
}

/// Pick the rename that matches the server extensions.
pub const fn choose(support_posix_rename: bool) -> PublishMethod {
    if support_posix_rename {
        PublishMethod::PosixRename
    } else {
        PublishMethod::RenameAside
    }
}

/// Publish the temp file. The final name is only ever written by a rename.
///
/// # Errors
/// Returns `Input` when no rename can complete; the temp file is left in
/// place for the caller to remove.
pub async fn publish(
    sftp: &Sftp,
    temp: &str,
    final_path: &str,
    run_id: &str,
    mtime: Option<u64>,
) -> Result<PublishMethod, UksftaError> {
    let mut fs = sftp.fs();
    let method = choose(sftp.support_posix_rename());
    match method {
        PublishMethod::PosixRename => {
            fs.rename(temp, final_path).await.map_err(|error| {
                UksftaError::Input(format!("cannot publish {final_path}: {error}"))
            })?;
        }
        PublishMethod::RenameAside => {
            let aside = plan_aside_name(final_path, run_id);
            let existed = fs.metadata(final_path).await.is_ok();
            if existed {
                fs.rename(final_path, &aside).await.map_err(|error| {
                    UksftaError::Input(format!("cannot move {final_path} aside: {error}"))
                })?;
            }
            if let Err(error) = fs.rename(temp, final_path).await {
                if existed {
                    let _ = fs.rename(&aside, final_path).await;
                }
                return Err(UksftaError::Input(format!(
                    "cannot publish {final_path}: {error}"
                )));
            }
            if existed {
                if let Err(error) = fs.remove_file(&aside).await {
                    eprintln!("Warning: cannot remove {aside}: {error}");
                }
            }
        }
    }

    if let Some(seconds) = mtime.and_then(|seconds| u32::try_from(seconds).ok()) {
        let Some(stamp) = UnixTimeStamp::from_raw(seconds) else {
            return Ok(method);
        };
        let mut builder = MetaDataBuilder::new();
        builder.time(stamp, stamp);
        if let Err(error) = fs.set_metadata(final_path, builder.create()).await {
            eprintln!("Warning: cannot set the timestamp on {final_path}: {error}");
        }
    }
    Ok(method)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rename_follows_the_server_extension() {
        assert_eq!(choose(true), PublishMethod::PosixRename);
        assert_eq!(choose(false), PublishMethod::RenameAside);
    }

    #[test]
    fn the_aside_name_is_not_loadable() {
        let aside = super::plan_aside_name("/r/a.pbo", "abcdef0123456789");
        assert!(aside.ends_with(".uksfta-old-abcdef0123456789"));
        assert!(!aside.ends_with(".pbo"));
    }
}
