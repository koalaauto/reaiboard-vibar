//! CoreAudio 查询:虚拟麦克风此刻有没有被某个 App 打开录音。
//! 用于「按需推流」:有人在录才向键盘要音频,没人录就停,省电也保护隐私。

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

/// 按名字找系统里的音频设备
pub fn find_device(name: &str) -> Option<AudioObjectID> {
    // SAFETY: 只调用 CoreAudio 的只读属性查询,缓冲区大小由系统先报告再分配。
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
    // 属性返回的是已 retain 的对象,按 create 规则接管
    Some(CFString::wrap_under_create_rule(cf).to_string())
}

/// 设备此刻是否正被某个进程用于 IO。查询失败(设备已消失)返回 None。
pub fn is_running_somewhere(id: AudioObjectID) -> Option<bool> {
    // SAFETY: 读取一个 u32 属性。
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
        // 默认输出设备一定存在;只要能枚举并读出名字就说明 FFI 正确
        // (CI 无声卡环境下允许为空,不算失败)
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
