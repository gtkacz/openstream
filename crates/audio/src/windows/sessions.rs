//! WASAPI session and Win32 process discovery for per-application capture.

use std::collections::{BTreeMap, BTreeSet};
use std::mem::size_of;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;

use brp_proto::constants::AUDIO_SOURCE_LIST_TIMEOUT;
use wasapi::{DeviceEnumerator, Direction, SessionState, initialize_mta};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_NO_MORE_FILES, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};

use crate::error::AudioError;
use crate::process_tree::{ProcessInfo, ProcessMap, application_root};
use crate::selection::{AppKey, AudioSource};

#[derive(Debug)]
struct SessionProcess {
    pid: u32,
    key: AppKey,
    label: String,
}

pub(super) fn list_sources(process_id: u32) -> Result<Vec<AudioSource>, AudioError> {
    let (tx, rx) = mpsc::channel();
    let thread = thread::Builder::new()
        .name("brp-audio-wasapi-list".into())
        .spawn(move || {
            let result = initialize_mta()
                .ok()
                .map_err(|error| AudioError::Windows(format!("CoInitializeEx: {error}")))
                .and_then(|_| enumerate_sessions(process_id))
                .map(collapse_sources);
            let _ = tx.send(result);
        })
        .map_err(|error| {
            AudioError::Windows(format!(
                "failed to spawn the WASAPI listing thread: {error}"
            ))
        })?;

    match rx.recv_timeout(AUDIO_SOURCE_LIST_TIMEOUT) {
        Ok(result) => {
            let _ = thread.join();
            result
        }
        Err(RecvTimeoutError::Timeout) => Err(AudioError::Windows(format!(
            "application enumeration did not finish within {AUDIO_SOURCE_LIST_TIMEOUT:?}"
        ))),
        Err(RecvTimeoutError::Disconnected) => {
            let _ = thread.join();
            Err(AudioError::Windows(
                "WASAPI application enumeration exited before reporting".into(),
            ))
        }
    }
}

pub(super) fn selected_roots(
    process_id: u32,
    selected: &BTreeSet<AppKey>,
) -> Result<BTreeSet<u32>, AudioError> {
    let sessions = enumerate_sessions(process_id)?;
    let processes = process_snapshot()?;
    Ok(sessions
        .iter()
        .filter(|session| selected.contains(&session.key))
        .filter_map(|session| application_root(session.pid, &session.key, &processes))
        .collect())
}

fn enumerate_sessions(process_id: u32) -> Result<Vec<SessionProcess>, AudioError> {
    let own_key = current_executable_key()?;
    let enumerator = DeviceEnumerator::new()
        .map_err(|error| windows_error("create device enumerator", error))?;
    let device = enumerator
        .get_default_device(&Direction::Render)
        .map_err(|error| windows_error("get default render device", error))?;
    let manager = device
        .get_iaudiosessionmanager()
        .map_err(|error| windows_error("get audio session manager", error))?;
    let sessions = manager
        .get_audiosessionenumerator()
        .map_err(|error| windows_error("enumerate audio sessions", error))?;
    let count = sessions
        .get_count()
        .map_err(|error| windows_error("count audio sessions", error))?;

    let mut found = Vec::new();
    for index in 0..count {
        let control = match sessions.get_session(index) {
            Ok(control) => control,
            Err(error) => {
                tracing::debug!(index, %error, "audio session disappeared during enumeration");
                continue;
            }
        };
        let state = match control.get_state() {
            Ok(state) => state,
            Err(error) => {
                tracing::debug!(index, %error, "could not read audio session state");
                continue;
            }
        };
        if state == SessionState::Expired {
            continue;
        }
        let pid = match control.get_process_id() {
            Ok(pid) if pid != 0 && pid != process_id => pid,
            Ok(_) => continue,
            Err(error) => {
                tracing::debug!(index, %error, "could not resolve audio session process");
                continue;
            }
        };
        let path = match process_executable(pid) {
            Ok(path) => path,
            Err(error) => {
                tracing::debug!(pid, %error, "audio session process identity is unavailable");
                continue;
            }
        };
        let key = AppKey::new(&path);
        if key == own_key {
            continue;
        }
        let reported_label = control.get_display_name().unwrap_or_default();
        let label = if reported_label.trim().is_empty() {
            key.as_str().to_string()
        } else {
            reported_label
        };
        found.push(SessionProcess { pid, key, label });
    }
    Ok(found)
}

fn collapse_sources(sessions: Vec<SessionProcess>) -> Vec<AudioSource> {
    let mut sources = BTreeMap::<AppKey, String>::new();
    for session in sessions {
        let fallback = session.key.as_str();
        sources
            .entry(session.key.clone())
            .and_modify(|label| {
                if label == fallback && session.label != fallback {
                    *label = session.label.clone();
                }
            })
            .or_insert(session.label);
    }
    sources
        .into_iter()
        .map(|(key, label)| AudioSource { key, label })
        .collect()
}

fn current_executable_key() -> Result<AppKey, AudioError> {
    let executable = std::env::current_exe()
        .map_err(|error| AudioError::Windows(format!("resolve brp executable: {error}")))?;
    Ok(AppKey::new(&executable.to_string_lossy()))
}

fn process_executable(pid: u32) -> Result<String, AudioError> {
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return Err(last_windows_error("OpenProcess"));
    }
    let handle = OwnedHandle(handle);
    let mut path = vec![0_u16; 32_768];
    let mut length = path.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(handle.0, 0, path.as_mut_ptr(), &mut length) };
    if ok == 0 {
        return Err(last_windows_error("QueryFullProcessImageNameW"));
    }
    Ok(String::from_utf16_lossy(&path[..length as usize]))
}

fn process_snapshot() -> Result<ProcessMap, AudioError> {
    let handle = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if handle == INVALID_HANDLE_VALUE {
        return Err(last_windows_error("CreateToolhelp32Snapshot"));
    }
    let handle = OwnedHandle(handle);
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    if unsafe { Process32FirstW(handle.0, &mut entry) } == 0 {
        return Err(last_windows_error("Process32FirstW"));
    }

    let mut processes = ProcessMap::new();
    loop {
        let end = entry
            .szExeFile
            .iter()
            .position(|character| *character == 0)
            .unwrap_or(entry.szExeFile.len());
        let executable = String::from_utf16_lossy(&entry.szExeFile[..end]);
        processes.insert(
            entry.th32ProcessID,
            ProcessInfo {
                parent: entry.th32ParentProcessID,
                key: AppKey::new(&executable),
            },
        );

        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        if unsafe { Process32NextW(handle.0, &mut entry) } == 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_NO_MORE_FILES as i32) {
                break;
            }
            return Err(AudioError::Windows(format!("Process32NextW: {error}")));
        }
    }
    Ok(processes)
}

struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

fn windows_error(call: &str, error: impl std::fmt::Display) -> AudioError {
    AudioError::Windows(format!("{call}: {error}"))
}

fn last_windows_error(call: &str) -> AudioError {
    AudioError::Windows(format!("{call}: {}", std::io::Error::last_os_error()))
}
