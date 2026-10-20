//! Composition snapshots, diffs, and state transitions.

use super::super::state::{ModMap, PublishState};
use super::super::text::sort_key;
use crate::lock::LockFile;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModChange {
    pub id: String,
    pub name: String,
    /// The Workshop update timestamp, shown for changed items.
    pub updated: Option<String>,
}

/// The difference between two compositions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChangeSet {
    pub added: Vec<ModChange>,
    pub updated: Vec<ModChange>,
    pub removed: Vec<ModChange>,
}

/// What the changelog section records.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Changes {
    /// The size of the first tracked composition, written once for the
    /// version that introduced the changelog.
    pub initial: Option<usize>,
    pub set: ChangeSet,
}

impl Changes {
    pub fn initial(count: usize) -> Self {
        Self {
            initial: Some(count),
            set: ChangeSet::default(),
        }
    }

    pub fn diff(set: ChangeSet) -> Self {
        Self { initial: None, set }
    }
}

/// The installed mods in a lock file, keyed by Workshop ID.
pub fn snapshot(lock: &LockFile) -> ModMap {
    lock.mods
        .iter()
        .filter(|(_, entry)| !entry.files.is_empty())
        .map(|(id, entry)| {
            (
                id.clone(),
                super::super::state::ModSnapshot {
                    name: entry.name.clone(),
                    updated: entry.updated.clone(),
                },
            )
        })
        .collect()
}

/// Compare the baseline with the current composition.
pub fn diff(baseline: &ModMap, current: &ModMap) -> ChangeSet {
    let mut changes = ChangeSet::default();

    for (id, now) in current {
        match baseline.get(id) {
            None => changes.added.push(change(id, now.name.clone())),
            Some(before) if before != now => changes.updated.push(change_with_update(
                id,
                now.name.clone(),
                now.updated.clone(),
            )),
            Some(_) => {}
        }
    }
    for (id, before) in baseline {
        if !current.contains_key(id) {
            changes.removed.push(change(id, before.name.clone()));
        }
    }

    sort(&mut changes.added);
    sort(&mut changes.updated);
    sort(&mut changes.removed);
    changes
}

pub(super) fn change(id: &str, name: String) -> ModChange {
    ModChange {
        id: id.to_string(),
        name,
        updated: None,
    }
}

pub(super) fn change_with_update(id: &str, name: String, updated: String) -> ModChange {
    ModChange {
        id: id.to_string(),
        name,
        updated: (!updated.trim().is_empty()).then_some(updated),
    }
}

fn sort(items: &mut [ModChange]) {
    items.sort_by(|a, b| {
        sort_key(&a.name)
            .cmp(&sort_key(&b.name))
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.id.cmp(&b.id))
    });
}

pub fn changes_for(previous: Option<&PublishState>, version: &str, current: &ModMap) -> Changes {
    match previous {
        None => Changes::initial(current.len()),
        Some(state) if state.project_version == version => Changes {
            initial: state.initial,
            set: diff(&state.baseline, current),
        },
        Some(state) => Changes::diff(diff(&state.last, current)),
    }
}

/// The state to save after a successful run. A version change starts a new
/// baseline at the composition from the previous run.
pub fn next_state(
    previous: Option<&PublishState>,
    version: &str,
    current: &ModMap,
) -> PublishState {
    let baseline = match previous {
        None => current.clone(),
        Some(state) if state.project_version == version => state.baseline.clone(),
        Some(state) => state.last.clone(),
    };
    let initial = match previous {
        None => Some(current.len()),
        Some(state) if state.project_version == version => state.initial,
        Some(_) => None,
    };
    PublishState {
        schema: 1,
        project_version: version.to_string(),
        baseline,
        last: current.clone(),
        initial,
    }
}
