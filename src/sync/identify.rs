use std::path::PathBuf;

use crate::error::UksftaError;
use crate::origin::{
    build_mod_dirs, build_pbo_index, resolve_pbo_from_index, self_workshop_id, ResolvedOrigin,
};
use crate::pbo::pbo_prefix;
use crate::pbo_hashes::PboHashes;
use crate::steam::find_all_workshop_caches;
use crate::util::{find_pbos, sanitize_for_terminal};
use crate::workshop_manifest::{shared_suffix, WorkshopIndex};

/// One PBO: what the prefix says and whether the manifest index has a name
/// candidate worth hashing.
struct Row {
    name: String,
    path: PathBuf,
    prefix: Option<ResolvedOrigin>,
    has_candidates: bool,
}

/// Show which PBOs in addons/ came from which Workshop mod.
pub fn identify() -> Result<(), UksftaError> {
    let caches = find_all_workshop_caches();
    if caches.is_empty() {
        return Err(UksftaError::Input("Workshop cache not found".to_string()));
    }

    let addons_dir = std::path::Path::new("addons");
    if !addons_dir.exists() {
        return Err(UksftaError::Input("No addons/ directory".to_string()));
    }

    let mut mod_dirs = build_mod_dirs(&caches);

    // If the current directory is itself a Workshop mod folder, exclude it.
    let self_id = self_workshop_id(&caches);
    if let Some(self_id) = self_id.as_deref() {
        mod_dirs.retain(|d| d.id != self_id);
    }

    println!("PBO Origins:");
    let index = build_pbo_index(&mod_dirs);

    // The depot manifest index holds the exact bytes of every installed
    // Workshop file. A name and size find candidates; a hash confirms one.
    // The item under investigation is dropped from it, or every file would
    // confirm against the pack's own manifest.
    let manifest_index = WorkshopIndex::load().map(|mut index| {
        if let Some(self_id) = self_id.as_deref() {
            index.exclude_item(self_id);
        }
        index
    });

    // First pass: prefix origin and candidate discovery read no file body.
    let mut rows = Vec::new();
    for pbo in find_pbos(addons_dir) {
        let Some(name) = pbo.file_name().map(|n| n.to_string_lossy().to_string()) else {
            continue;
        };
        let target_prefix = pbo_prefix(&pbo);
        let candidates = index.candidates(&name);
        let prefix_origin = resolve_pbo_from_index(&pbo, candidates, target_prefix.as_deref());
        let has_candidates = manifest_index
            .as_ref()
            .is_some_and(|index| !index.candidates_for(&pbo).is_empty());
        rows.push(Row {
            name,
            path: pbo,
            prefix: prefix_origin,
            has_candidates,
        });
    }

    // One bounded batch serves every candidate. An unchanged file is read
    // from the hash cache instead.
    let mut hashes = PboHashes::load(&PboHashes::default_path());
    let hash_paths: Vec<PathBuf> = rows
        .iter()
        .filter(|row| row.has_candidates)
        .map(|row| row.path.clone())
        .collect();
    let mut digests = hashes.digests(&hash_paths).into_iter();

    for row in &rows {
        if row.has_candidates {
            let index = manifest_index
                .as_ref()
                .expect("a candidate needs a manifest index");
            match digests.next().flatten() {
                Some((sha, size)) => {
                    let entries = index.lookup(&sha, size);
                    if !entries.is_empty() {
                        let hint = row.prefix.as_ref().map(|origin| origin.id.as_str());
                        let chosen = index
                            .pick(entries, hint)
                            .expect("a confirmed match has at least one entry");
                        println!(
                            "  {} -> Workshop {} [content hash: {}{}]",
                            row.name,
                            chosen.published_id,
                            sanitize_for_terminal(&chosen.path),
                            shared_suffix(entries)
                        );
                        continue;
                    }
                }
                // The name is a candidate but the bytes cannot be read, so
                // the candidates stay unconfirmed.
                None => {
                    let candidates = index.candidates_for(&row.path);
                    let ids = candidates.ids().join(", ");
                    let basis = if candidates.exact_size {
                        "name and size"
                    } else {
                        "name only"
                    };
                    println!(
                        "  {} -> candidates {} [{}; the file could not be read]",
                        row.name, ids, basis
                    );
                    continue;
                }
            }
        }

        let origin = row
            .prefix
            .as_ref()
            .map(|resolved| resolved.id.as_str())
            .unwrap_or("Unknown");
        println!("  {} -> Workshop {}", row.name, origin);
    }
    hashes.finish();
    Ok(())
}
