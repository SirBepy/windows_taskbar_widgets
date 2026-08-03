const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const VALUE_NAME: &str = "Taskbar Widgets";

#[cfg(target_os = "windows")]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(target_os = "windows")]
fn open_run_key(sam: windows_sys::Win32::System::Registry::REG_SAM_FLAGS) -> Option<windows_sys::Win32::System::Registry::HKEY> {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{RegOpenKeyExW, HKEY, HKEY_CURRENT_USER};
    let subkey = wide(RUN_KEY);
    let mut hkey: HKEY = std::ptr::null_mut();
    let ok = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, subkey.as_ptr(), 0, sam, &mut hkey) };
    (ok == ERROR_SUCCESS).then_some(hkey)
}

/// True only when the value exists and holds a non-empty string.
#[cfg(target_os = "windows")]
#[tauri::command]
pub fn get_autostart() -> bool {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{RegCloseKey, RegQueryValueExW, KEY_QUERY_VALUE};
    let Some(hkey) = open_run_key(KEY_QUERY_VALUE) else { return false };
    let value_name = wide(VALUE_NAME);
    let mut data_len: u32 = 0;
    let ok = unsafe {
        RegQueryValueExW(
            hkey,
            value_name.as_ptr(),
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut data_len,
        )
    };
    unsafe { RegCloseKey(hkey) };
    ok == ERROR_SUCCESS && data_len > 0
}

#[cfg(target_os = "windows")]
#[tauri::command]
pub fn set_autostart(enabled: bool) -> Result<(), String> {
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegDeleteValueW, RegSetValueExW, KEY_SET_VALUE, REG_SZ,
    };
    // Resolved before the key is opened so an error here can't leak the HKEY.
    let exe = if enabled {
        Some(std::env::current_exe().map_err(|e| e.to_string())?)
    } else {
        None
    };
    let Some(hkey) = open_run_key(KEY_SET_VALUE) else {
        return Err("failed to open Run key".into());
    };
    let value_name = wide(VALUE_NAME);
    let result = if let Some(exe) = exe {
        let data = wide(&format!("\"{}\"", exe.display()));
        // REG_SZ data is the raw UTF-16 bytes (incl. NUL terminator), not the Vec<u16> itself.
        let bytes = unsafe { std::slice::from_raw_parts(data.as_ptr().cast::<u8>(), data.len() * 2) };
        unsafe { RegSetValueExW(hkey, value_name.as_ptr(), 0, REG_SZ, bytes.as_ptr(), bytes.len() as u32) }
    } else {
        // Deleting an absent value returns ERROR_FILE_NOT_FOUND; disabling twice must not error.
        match unsafe { RegDeleteValueW(hkey, value_name.as_ptr()) } {
            ERROR_FILE_NOT_FOUND => ERROR_SUCCESS,
            e => e,
        }
    };
    unsafe { RegCloseKey(hkey) };
    if result == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(format!("registry error {result}"))
    }
}

#[cfg(not(target_os = "windows"))]
#[tauri::command]
pub fn get_autostart() -> bool {
    false
}

#[cfg(not(target_os = "windows"))]
#[tauri::command]
pub fn set_autostart(_enabled: bool) -> Result<(), String> {
    Ok(())
}
