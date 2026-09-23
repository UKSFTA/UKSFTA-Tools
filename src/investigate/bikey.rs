//! Signing-key name detection.

use std::fs;
use std::path::Path;

/// Scan for .bikey files in the mod pack's parent directory. The key
/// name often identifies the author team (e.g. "TFB.bistkeys",
/// "ZSquadron.bistkeys"). Returns a list of key base names.
pub(crate) fn scan_bikey_names(addons_dir: &Path) -> Vec<String> {
    // Bikey files are typically in the parent of the addons/ folder
    // (i.e. the Workshop item root: workshop/content/107410/<id>/keys/)
    let pack_root = addons_dir.parent().unwrap_or(addons_dir);
    let keys_dir = pack_root.join("keys");
    let mut names = Vec::new();

    if let Ok(entries) = fs::read_dir(&keys_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".bikey") || name.ends_with(".bisign") {
                let base = name
                    .rsplit_once('.')
                    .map(|(b, _)| b.to_string())
                    .unwrap_or(name);
                if !names.contains(&base) {
                    names.push(base);
                }
            }
        }
    }
    names
}
