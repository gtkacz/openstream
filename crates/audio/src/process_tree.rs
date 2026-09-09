//! Pure process-ancestry rules used by Windows per-application capture.

use std::collections::{BTreeMap, BTreeSet};

use crate::AppKey;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProcessInfo {
    pub parent: u32,
    pub key: AppKey,
}

pub(crate) type ProcessMap = BTreeMap<u32, ProcessInfo>;

/// Finds the topmost ancestor that still belongs to the same executable.
///
/// WASAPI include mode captures a whole process tree. Collapsing same-name ancestors prevents a
/// browser renderer and its browser parent from being captured twice, while a differently named
/// launcher stops the walk and keeps independent application instances independent.
pub(crate) fn application_root(pid: u32, key: &AppKey, processes: &ProcessMap) -> Option<u32> {
    let first = processes.get(&pid)?;
    if &first.key != key {
        return None;
    }

    let mut root = pid;
    let mut seen = BTreeSet::from([pid]);
    loop {
        let parent = processes.get(&root)?.parent;
        let Some(parent_info) = processes.get(&parent) else {
            return Some(root);
        };
        if &parent_info.key != key || !seen.insert(parent) {
            return Some(root);
        }
        root = parent;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(parent: u32, name: &str) -> ProcessInfo {
        ProcessInfo {
            parent,
            key: AppKey::new(name),
        }
    }

    #[test]
    fn renderer_collapses_onto_the_browser_root() {
        let processes = BTreeMap::from([
            (10, info(1, "browser.exe")),
            (11, info(10, "browser.exe")),
            (12, info(11, "browser.exe")),
        ]);

        assert_eq!(
            application_root(12, &AppKey::new("browser.exe"), &processes),
            Some(10)
        );
    }

    #[test]
    fn independent_instances_keep_independent_roots() {
        let processes = BTreeMap::from([
            (10, info(1, "browser.exe")),
            (11, info(10, "browser.exe")),
            (20, info(1, "browser.exe")),
            (21, info(20, "browser.exe")),
        ]);

        assert_eq!(
            application_root(11, &AppKey::new("browser.exe"), &processes),
            Some(10)
        );
        assert_eq!(
            application_root(21, &AppKey::new("browser.exe"), &processes),
            Some(20)
        );
    }

    #[test]
    fn differently_named_launcher_stops_the_walk() {
        let processes = BTreeMap::from([
            (5, info(1, "launcher.exe")),
            (10, info(5, "game.exe")),
            (11, info(10, "game.exe")),
        ]);

        assert_eq!(
            application_root(11, &AppKey::new("game.exe"), &processes),
            Some(10)
        );
    }

    #[test]
    fn missing_or_mismatched_process_fails_closed() {
        let processes = BTreeMap::from([(10, info(1, "game.exe"))]);

        assert_eq!(
            application_root(99, &AppKey::new("game.exe"), &processes),
            None
        );
        assert_eq!(
            application_root(10, &AppKey::new("chat.exe"), &processes),
            None
        );
    }
}
