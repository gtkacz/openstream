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

/// Which selected applications can be captured, and which have to be held back.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct CapturePlan {
    pub roots: BTreeMap<u32, AppKey>,
    pub held_back: BTreeSet<AppKey>,
}

/// WASAPI can only include a process together with everything it started, so a selected application
/// is captured only when its tree holds no other application's audio and not this process. Anything
/// else would reach the room unchosen, or twice when the other application is selected as well.
///
/// shortcut: a child that starts playing is heard until the next poll notices it, up to
/// `AUDIO_SESSION_POLL_INTERVAL`, because Windows offers no way to capture a single process.
pub(crate) fn capture_plan(
    sessions: &[(u32, AppKey)],
    own_process: u32,
    selected: &BTreeSet<AppKey>,
    processes: &ProcessMap,
) -> CapturePlan {
    let mut plan = CapturePlan::default();
    for (pid, key) in sessions.iter().filter(|(_, key)| selected.contains(key)) {
        let Some(root) = application_root(*pid, key, processes) else {
            continue;
        };
        let shares_its_tree = in_tree(own_process, root, processes)
            || sessions
                .iter()
                .any(|(other, other_key)| other_key != key && in_tree(*other, root, processes));
        if shares_its_tree {
            plan.held_back.insert(key.clone());
        } else {
            plan.roots.insert(root, key.clone());
        }
    }
    plan
}

/// Whether `pid` is `root` or descends from it. A parent id can outlive its process and be reused,
/// which at worst holds an application back rather than sharing too much.
fn in_tree(pid: u32, root: u32, processes: &ProcessMap) -> bool {
    let mut current = pid;
    let mut seen = BTreeSet::new();
    while seen.insert(current) {
        if current == root {
            return true;
        }
        match processes.get(&current) {
            Some(info) => current = info.parent,
            None => return false,
        }
    }
    false
}

/// Whether another OpenStream window has an audio session. Windows exclude mode leaves out one process
/// tree, and a window opened by a link is not in this one's, so its room would be heard in this room.
pub(crate) fn another_window_audible(
    sessions: &[(u32, AppKey)],
    own_process: u32,
    own_key: &AppKey,
) -> bool {
    sessions
        .iter()
        .any(|(pid, key)| key == own_key && *pid != own_process)
}

/// What the user is told about the applications a plan holds back.
pub(crate) fn held_back_notice(held_back: &BTreeSet<AppKey>) -> Option<String> {
    if held_back.is_empty() {
        return None;
    }
    let names: Vec<&str> = held_back.iter().map(AppKey::as_str).collect();
    Some(format!(
        "Not sharing {}: other audio it started would be shared too",
        names.join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_another_process_of_this_application_counts_as_another_window() {
        let own = AppKey::new("brp.exe");
        assert!(!another_window_audible(&[], 7, &own));
        assert!(
            !another_window_audible(&[session(7, "brp.exe")], 7, &own),
            "this window's own playback is already excluded"
        );
        assert!(!another_window_audible(
            &[session(8, "browser.exe")],
            7,
            &own
        ));
        assert!(another_window_audible(
            &[session(7, "brp.exe"), session(8, "brp.exe")],
            7,
            &own
        ));
    }

    #[test]
    fn the_notice_names_what_was_held_back_and_says_nothing_otherwise() {
        assert_eq!(held_back_notice(&BTreeSet::new()), None);
        let notice = held_back_notice(&keys(&["launcher.exe", "other.exe"])).unwrap();
        assert!(notice.contains("launcher.exe, other.exe"), "{notice}");
    }

    fn keys(names: &[&str]) -> BTreeSet<AppKey> {
        names.iter().map(|name| AppKey::new(name)).collect()
    }

    fn session(pid: u32, name: &str) -> (u32, AppKey) {
        (pid, AppKey::new(name))
    }

    /// A launcher that started a game, and a browser nothing else lives under.
    fn desktop() -> ProcessMap {
        BTreeMap::from([
            (5, info(1, "launcher.exe")),
            (10, info(5, "game.exe")),
            (20, info(1, "browser.exe")),
            (21, info(20, "browser.exe")),
        ])
    }

    #[test]
    fn a_tree_holding_only_the_selected_application_is_captured() {
        let plan = capture_plan(
            &[session(21, "browser.exe"), session(10, "game.exe")],
            99,
            &keys(&["browser.exe"]),
            &desktop(),
        );
        assert_eq!(
            plan.roots,
            BTreeMap::from([(20, AppKey::new("browser.exe"))])
        );
        assert!(plan.held_back.is_empty());
    }

    #[test]
    fn a_launcher_whose_child_plays_unselected_audio_is_held_back() {
        let plan = capture_plan(
            &[session(5, "launcher.exe"), session(10, "game.exe")],
            99,
            &keys(&["launcher.exe"]),
            &desktop(),
        );
        assert!(plan.roots.is_empty(), "the game would reach the room");
        assert_eq!(plan.held_back, keys(&["launcher.exe"]));
    }

    #[test]
    fn a_child_selected_with_its_parent_is_captured_once_at_its_own_volume() {
        let plan = capture_plan(
            &[session(5, "launcher.exe"), session(10, "game.exe")],
            99,
            &keys(&["launcher.exe", "game.exe"]),
            &desktop(),
        );
        assert_eq!(plan.roots, BTreeMap::from([(10, AppKey::new("game.exe"))]));
        assert_eq!(plan.held_back, keys(&["launcher.exe"]));
    }

    #[test]
    fn a_tree_this_process_descends_from_is_held_back() {
        let mut processes = desktop();
        processes.insert(30, info(5, "brp.exe"));
        let plan = capture_plan(
            &[session(5, "launcher.exe")],
            30,
            &keys(&["launcher.exe"]),
            &processes,
        );
        assert!(plan.roots.is_empty());
        assert_eq!(plan.held_back, keys(&["launcher.exe"]));
    }

    #[test]
    fn another_openstream_window_under_a_selected_application_holds_it_back() {
        let mut processes = desktop();
        processes.insert(30, info(5, "brp.exe"));
        let plan = capture_plan(
            &[session(5, "launcher.exe"), session(30, "brp.exe")],
            99,
            &keys(&["launcher.exe"]),
            &processes,
        );
        assert_eq!(plan.held_back, keys(&["launcher.exe"]));
    }

    #[test]
    fn a_parent_id_loop_ends_the_walk() {
        let processes = BTreeMap::from([(1, info(2, "a.exe")), (2, info(1, "b.exe"))]);
        assert!(!in_tree(1, 3, &processes));
    }

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
