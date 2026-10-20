use serde::Serialize;
#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameState {
    pub running: bool,
    pub foreground: bool,
    /// Miliastra is in front in exclusive fullscreen, where notifications can't show.
    pub exclusive_fullscreen: bool,
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}
#[cfg(windows)]
pub fn game_state() -> GameState {
    use windows::Win32::{
        Foundation::{CloseHandle, RECT},
        System::Diagnostics::ToolHelp::*,
        UI::WindowsAndMessaging::*,
    };
    let mut out = GameState::default();
    unsafe {
        let hwnd = GetForegroundWindow();
        let mut foreground_pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut foreground_pid));
        if let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
            let mut entry = PROCESSENTRY32W {
                dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };
            if Process32FirstW(snapshot, &mut entry).is_ok() {
                loop {
                    let end = entry
                        .szExeFile
                        .iter()
                        .position(|c| *c == 0)
                        .unwrap_or(entry.szExeFile.len());
                    let name = String::from_utf16_lossy(&entry.szExeFile[..end]).to_lowercase();
                    if ["genshinimpact.exe", "yuanshen.exe"].contains(&name.as_str()) {
                        out.running = true;
                        if entry.th32ProcessID == foreground_pid {
                            out.foreground = true;
                            let mut r = RECT::default();
                            if GetWindowRect(hwnd, &mut r).is_ok() {
                                out.left = r.left;
                                out.top = r.top;
                                out.right = r.right;
                                out.bottom = r.bottom;
                            }
                        }
                    }
                    if Process32NextW(snapshot, &mut entry).is_err() {
                        break;
                    }
                }
            }
            let _ = CloseHandle(snapshot);
        }
    }
    out.exclusive_fullscreen = out.foreground && exclusive_fullscreen();
    out
}
/// Process IDs of running Miliastra (Genshin Impact) clients.
#[cfg(windows)]
pub fn game_process_ids() -> Vec<u32> {
    use windows::Win32::{Foundation::CloseHandle, System::Diagnostics::ToolHelp::*};
    let mut ids = vec![];
    unsafe {
        if let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
            let mut entry = PROCESSENTRY32W {
                dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };
            if Process32FirstW(snapshot, &mut entry).is_ok() {
                loop {
                    let end = entry
                        .szExeFile
                        .iter()
                        .position(|c| *c == 0)
                        .unwrap_or(entry.szExeFile.len());
                    let name = String::from_utf16_lossy(&entry.szExeFile[..end]).to_lowercase();
                    if ["genshinimpact.exe", "yuanshen.exe"].contains(&name.as_str()) {
                        ids.push(entry.th32ProcessID);
                    }
                    if Process32NextW(snapshot, &mut entry).is_err() {
                        break;
                    }
                }
            }
            let _ = CloseHandle(snapshot);
        }
    }
    ids
}
#[cfg(not(windows))]
pub fn game_process_ids() -> Vec<u32> {
    vec![]
}
/// When the earliest of these game processes started.
#[cfg(windows)]
pub fn game_started_at(ids: &[u32]) -> Option<std::time::SystemTime> {
    use windows::Win32::{
        Foundation::{CloseHandle, FILETIME},
        System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
    };
    let mut earliest: Option<std::time::SystemTime> = None;
    for id in ids {
        unsafe {
            let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, *id) else {
                return None;
            };
            let (mut created, mut exited, mut kernel, mut user) = (
                FILETIME::default(),
                FILETIME::default(),
                FILETIME::default(),
                FILETIME::default(),
            );
            let ok = GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user);
            let _ = CloseHandle(handle);
            ok.ok()?;
            // FILETIME counts 100 ns intervals since 1601-01-01.
            let ticks = ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64;
            let unix = ticks.checked_sub(116_444_736_000_000_000)?;
            let at = std::time::UNIX_EPOCH + std::time::Duration::from_nanos(unix * 100);
            earliest = Some(earliest.map_or(at, |e| e.min(at)));
        }
    }
    earliest
}
#[cfg(not(windows))]
pub fn game_started_at(_ids: &[u32]) -> Option<std::time::SystemTime> {
    None
}
/// A Direct3D app is in front in exclusive fullscreen, which hides notifications and
/// covers topmost windows.
#[cfg(windows)]
fn exclusive_fullscreen() -> bool {
    use windows::Win32::UI::Shell::{SHQueryUserNotificationState, QUNS_RUNNING_D3D_FULL_SCREEN};
    unsafe { SHQueryUserNotificationState().is_ok_and(|s| s == QUNS_RUNNING_D3D_FULL_SCREEN) }
}
#[cfg(not(windows))]
pub fn game_state() -> GameState {
    GameState::default()
}

/// Where the notification goes on screen (bottom-right of the work area).
pub fn notification_position(
    left: i32,
    top: i32,
    width: u32,
    height: u32,
    notification_width: u32,
    notification_height: u32,
    scale: f64,
) -> (i32, i32) {
    let margin = (24.0 * scale) as i32;
    (
        (left + width as i32 - notification_width as i32 - margin).max(left),
        (top + height as i32 - notification_height as i32 - margin).max(top),
    )
}
#[cfg(windows)]
pub fn foreground_center() -> Option<(i32, i32)> {
    use windows::Win32::{
        Foundation::RECT,
        UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowRect},
    };
    unsafe {
        let mut r = RECT::default();
        GetWindowRect(GetForegroundWindow(), &mut r).ok()?;
        Some(((r.left + r.right) / 2, (r.top + r.bottom) / 2))
    }
}
#[cfg(not(windows))]
pub fn foreground_center() -> Option<(i32, i32)> {
    None
}
#[cfg(test)]
mod notification_tests {
    use super::*;
    #[test]
    fn anchors_to_monitor_work_area_including_scaling_and_negative_origins() {
        assert_eq!(
            notification_position(0, 0, 1920, 1040, 420, 170, 1.0),
            (1476, 846)
        );
        assert_eq!(
            notification_position(-2560, 100, 2560, 1400, 840, 340, 2.0),
            (-888, 1112)
        );
        assert_eq!(notification_position(0, 0, 300, 200, 420, 170, 1.0), (0, 6));
    }
}
