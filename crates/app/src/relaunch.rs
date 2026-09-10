//! Starting the updated binary after this process leaves: back into the same room, with the
//! flags this launch was given.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;

use brp_update::Install;

use crate::cli::WindowArgs;

#[cfg(test)]
mod tests {
    use super::*;

    fn os(parts: &[&str]) -> Vec<OsString> {
        parts.iter().map(OsString::from).collect()
    }

    #[test]
    fn in_a_room_the_new_process_joins_it_with_each_present_flag() {
        let args = WindowArgs {
            nickname: Some("alice".into()),
            fps: Some(30),
            no_relay: true,
        };
        assert_eq!(
            relaunch_args(Some("brpticket"), &args),
            os(&[
                "join",
                "brpticket",
                "--nickname",
                "alice",
                "--fps",
                "30",
                "--no-relay"
            ])
        );
        assert_eq!(
            relaunch_args(Some("brpticket"), &WindowArgs::default()),
            os(&["join", "brpticket"])
        );
    }

    #[test]
    fn from_the_start_screen_the_new_process_takes_no_arguments() {
        let args = WindowArgs {
            nickname: Some("alice".into()),
            fps: Some(30),
            no_relay: true,
        };
        assert!(relaunch_args(None, &args).is_empty());
    }

    #[test]
    fn the_relaunch_runs_the_exe_of_the_install_captured_at_startup() {
        let install = Install::at(PathBuf::from("/opt/brp/brp")).unwrap();
        let relaunch = Relaunch::new(&install, Some("t"), &WindowArgs::default());
        assert_eq!(relaunch.exe, PathBuf::from("/opt/brp/brp"));
        assert_eq!(relaunch.args, os(&["join", "t"]));
    }
}

/// The process to start once this one has left the room.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relaunch {
    /// The installed binary, which the swap has already repointed at the new version.
    pub exe: PathBuf,
    /// The whole command line for the new process, already built; nothing is added at spawn.
    pub args: Vec<OsString>,
}

impl Relaunch {
    /// `install.exe` is the path captured at startup, which the swap has just repointed at the
    /// new binary; `current_exe()` here would name the renamed `.old` file on Linux.
    pub fn new(install: &Install, ticket: Option<&str>, args: &WindowArgs) -> Self {
        Self {
            exe: install.exe.clone(),
            args: relaunch_args(ticket, args),
        }
    }

    /// Starts the new process and returns at once. A failure can only be logged: the window is
    /// gone, and the update is already applied, so the next manual launch runs the new version.
    pub fn spawn(&self) {
        match Command::new(&self.exe).args(&self.args).spawn() {
            Ok(child) => {
                tracing::info!(pid = child.id(), exe = %self.exe.display(), "relaunched after the update");
            }
            Err(error) => {
                tracing::error!(%error, exe = %self.exe.display(), "could not relaunch after the update");
                eprintln!("error: could not relaunch {}: {error}", self.exe.display());
            }
        }
    }
}

/// `join <ticket>` with this launch's flags when a room was open. A bare `brp` takes no flags,
/// so the start screen relaunches without any.
pub fn relaunch_args(ticket: Option<&str>, args: &WindowArgs) -> Vec<OsString> {
    let Some(ticket) = ticket else {
        return Vec::new();
    };
    let mut out = vec![OsString::from("join"), OsString::from(ticket)];
    if let Some(nickname) = &args.nickname {
        out.push("--nickname".into());
        out.push(nickname.into());
    }
    if let Some(fps) = args.fps {
        out.push("--fps".into());
        out.push(fps.to_string().into());
    }
    if args.no_relay {
        out.push("--no-relay".into());
    }
    out
}
