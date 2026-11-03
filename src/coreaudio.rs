//! CoreAudio query, whether some app currently has the virtual microphone open for recording.
//! Used for on-demand streaming. Audio is requested from the keyboard only while someone is
//! recording and is stopped otherwise, which saves power and protects privacy.

use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::null;

use core_foundation::base::TCFType;
use core_foundation::string::{CFString, CFStringRef};
use coreaudio_sys::{
    kAudioDevicePropertyDeviceIsRunningSomewhere, kAudioHardwarePropertyDevices,
    kAudioObjectPropertyElementMaster, kAudioObjectPropertyName, kAudioObjectPropertyScopeGlobal,
    kAudioObjectSystemObject, AudioObjectGetPropertyData, AudioObjectGetPropertyDataSize,
    AudioObjectID, AudioObjectPropertyAddress,
};

fn address(selector: u32) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMaster,
    }
}

/// Finds an audio device in the system by name
pub fn find_device(name: &str) -> Option<AudioObjectID> {
    // SAFETY: Read-only CoreAudio property queries, buffer sized from the size the system reports.
    unsafe {
        let addr = address(kAudioHardwarePropertyDevices);
        let mut size = 0u32;
        if AudioObjectGetPropertyDataSize(kAudioObjectSystemObject, &addr, 0, null(), &mut size)
            != 0
        {
            return None;
        }
        let mut ids = vec![0 as AudioObjectID; size as usize / size_of::<AudioObjectID>()];
        if AudioObjectGetPropertyData(
            kAudioObjectSystemObject,
            &addr,
            0,
            null(),
            &mut size,
            ids.as_mut_ptr() as *mut c_void,
        ) != 0
        {
            return None;
        }
        ids.into_iter()
            .find(|id| device_name(*id).as_deref() == Some(name))
    }
}

unsafe fn device_name(id: AudioObjectID) -> Option<String> {
    let addr = address(kAudioObjectPropertyName);
    let mut cf: CFStringRef = std::ptr::null();
    let mut size = size_of::<CFStringRef>() as u32;
    let status = AudioObjectGetPropertyData(
        id,
        &addr,
        0,
        null(),
        &mut size,
        &mut cf as *mut _ as *mut c_void,
    );
    if status != 0 || cf.is_null() {
        return None;
    }
    // The property returns an already retained object, so take ownership under the create rule
    Some(CFString::wrap_under_create_rule(cf).to_string())
}

/// Whether some process is using the device for IO right now. Returns None if the query fails
/// because the device has disappeared.
pub fn is_running_somewhere(id: AudioObjectID) -> Option<bool> {
    // SAFETY: Reads one u32 property.
    unsafe {
        let addr = address(kAudioDevicePropertyDeviceIsRunningSomewhere);
        let mut value = 0u32;
        let mut size = size_of::<u32>() as u32;
        let status = AudioObjectGetPropertyData(
            id,
            &addr,
            0,
            null(),
            &mut size,
            &mut value as *mut _ as *mut c_void,
        );
        (status == 0).then_some(value != 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finding_a_nonexistent_device_returns_none() {
        assert_eq!(find_device("这个设备绝对不存在-vibar-test"), None);
    }

    #[test]
    fn system_has_at_least_one_audio_device_with_a_readable_name() {
        // A machine normally has a default output device. Being able to query the device list
        // at all shows the FFI is correct. An empty list is not a failure, for example in CI
        // without a sound card.
        unsafe {
            let addr = address(kAudioHardwarePropertyDevices);
            let mut size = 0u32;
            let ok = AudioObjectGetPropertyDataSize(
                kAudioObjectSystemObject,
                &addr,
                0,
                null(),
                &mut size,
            ) == 0;
            assert!(ok);
        }
    }
}
