//! Which processes are playing audio: CoreAudio's process objects (macOS 14.2 and later), which
//! say per process whether it is running output right now.

use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::NonNull;

use objc2_app_kit::NSRunningApplication;
use objc2_core_audio::{
    AudioObjectGetPropertyData, AudioObjectGetPropertyDataSize, AudioObjectID,
    AudioObjectPropertyAddress, kAudioHardwarePropertyProcessObjectList,
    kAudioObjectPropertyElementMain, kAudioObjectPropertyScopeGlobal, kAudioObjectSystemObject,
    kAudioProcessPropertyBundleID, kAudioProcessPropertyIsRunningOutput, kAudioProcessPropertyPID,
};
use objc2_core_foundation::{CFRetained, CFString};

use crate::error::AudioError;
use crate::macos_model::AudioProcess;

/// Every process CoreAudio tracks, with whether it is playing. Errors on systems older than
/// 14.2, which do not have process objects; the caller falls back to the running applications.
pub(super) fn audio_processes() -> Result<Vec<AudioProcess>, AudioError> {
    let objects: Vec<AudioObjectID> = array(
        kAudioObjectSystemObject as AudioObjectID,
        kAudioHardwarePropertyProcessObjectList,
    )?;
    Ok(objects
        .into_iter()
        .filter_map(|object| {
            let pid: i32 = scalar(object, kAudioProcessPropertyPID).ok()?;
            let running: u32 = scalar(object, kAudioProcessPropertyIsRunningOutput).unwrap_or(0);
            let bundle_id = string(object, kAudioProcessPropertyBundleID);
            Some(AudioProcess {
                pid,
                bundle_id,
                label: application_name(pid),
                running_output: running != 0,
            })
        })
        .collect())
}

/// The name the Dock shows for `pid`, when it is an application.
pub(super) fn application_name(pid: i32) -> Option<String> {
    NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
        .and_then(|app| app.localizedName())
        .map(|name| name.to_string())
}

fn address(selector: u32) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain,
    }
}

fn status(call: &str, code: i32) -> Result<(), AudioError> {
    if code == 0 {
        Ok(())
    } else {
        Err(AudioError::MacOs(format!(
            "{call} failed with status {code}"
        )))
    }
}

fn scalar<T: Copy + Default>(object: AudioObjectID, selector: u32) -> Result<T, AudioError> {
    let mut value = T::default();
    let mut size = size_of::<T>() as u32;
    let mut address = address(selector);
    // SAFETY: `value` is a live, correctly sized buffer and `size` says how big it is.
    let code = unsafe {
        AudioObjectGetPropertyData(
            object,
            NonNull::from(&mut address),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::from(&mut value).cast::<c_void>(),
        )
    };
    status("AudioObjectGetPropertyData", code)?;
    Ok(value)
}

fn array<T: Copy + Default>(object: AudioObjectID, selector: u32) -> Result<Vec<T>, AudioError> {
    let mut address = address(selector);
    let mut size = 0u32;
    // SAFETY: plain size query on a live address.
    let code = unsafe {
        AudioObjectGetPropertyDataSize(
            object,
            NonNull::from(&mut address),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
        )
    };
    status("AudioObjectGetPropertyDataSize", code)?;
    let count = size as usize / size_of::<T>();
    if count == 0 {
        return Ok(Vec::new());
    }
    let mut values = vec![T::default(); count];
    let mut size = (count * size_of::<T>()) as u32;
    // SAFETY: `values` has room for `size` bytes; CoreAudio writes at most that and reports how
    // much it wrote.
    let code = unsafe {
        AudioObjectGetPropertyData(
            object,
            NonNull::from(&mut address),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::new_unchecked(values.as_mut_ptr()).cast::<c_void>(),
        )
    };
    status("AudioObjectGetPropertyData", code)?;
    values.truncate(size as usize / size_of::<T>());
    Ok(values)
}

fn string(object: AudioObjectID, selector: u32) -> Option<String> {
    // A `CFStringRef`, read as an address-sized integer since pointers have no default.
    let raw: usize = scalar(object, selector).ok()?;
    let raw = NonNull::new(raw as *mut CFString)?;
    // SAFETY: CoreAudio returns a retained CFString the caller owns.
    let value = unsafe { CFRetained::from_raw(raw) };
    let text = value.to_string();
    (!text.is_empty()).then_some(text)
}
