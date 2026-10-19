/// Known Windows shell/system surfaces that routinely present a full-monitor rect
/// during ordinary use (opening Start, invoking Search, Xbox Game Bar, or Windows'
/// own screen-recording entry points) without being a real fullscreen app. Before
/// this exclusion, opening the Start menu or starting a screen recording got
/// misdetected as fullscreen-covering that monitor and triggered `move_on_fullscreen`,
/// relocating that monitor's strip onto the other one for no reason (reported
/// 2026-10-19).
pub(crate) fn is_excluded_process_exe(exe_name: &str) -> bool {
    const EXCLUDED: &[&str] = &[
        "StartMenuExperienceHost.exe",
        "SearchHost.exe",
        "ShellExperienceHost.exe",
        "ShellHost.exe",
        "TextInputHost.exe",
        "GameBar.exe",
        "GameBarFTServer.exe",
        "GameBarPresenceWriter.exe",
        "SnippingTool.exe",
        "ScreenClippingHost.exe",
    ];
    EXCLUDED.iter().any(|n| n.eq_ignore_ascii_case(exe_name))
}

/// Resolves a pid's own exe file name (no path) so `is_excluded_process_exe` can
/// recognize it. OpenProcess+Query is 3 syscalls, so callers gate this behind the
/// rare conditions (a covering rect, a presentation-state hit) rather than running
/// it for every window.
#[cfg(target_os = "windows")]
pub(crate) fn process_exe_name(pid: u32) -> Option<String> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut buf = [0u16; 512];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut len) != 0;
        CloseHandle(handle);
        if !ok {
            return None;
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        path.rsplit(['\\', '/']).next().map(str::to_string)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_shell_surface_names_are_excluded() {
        assert!(is_excluded_process_exe("StartMenuExperienceHost.exe"));
        assert!(is_excluded_process_exe("SearchHost.exe"));
        assert!(is_excluded_process_exe("GameBar.exe"));
        assert!(is_excluded_process_exe("ScreenClippingHost.exe"));
    }

    #[test]
    fn match_is_case_insensitive() {
        assert!(is_excluded_process_exe("startmenuexperiencehost.EXE"));
    }

    #[test]
    fn unrelated_exe_name_is_not_excluded() {
        assert!(!is_excluded_process_exe("SomeGame.exe"));
    }
}
