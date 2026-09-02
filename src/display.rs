use windows::Win32::Foundation::{POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, HMONITOR, MONITORINFO,
    MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY,
};
use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, GetForegroundWindow};

/// Returns the top-left position to place a window of the given size, horizontally
/// centered and vertically toward the top of the work area of the display under the
/// mouse cursor (`DisplayTarget::Cursor`, the default). Returns `None` on failure, in
/// which case the caller falls back to whatever default position the OS picks.
pub fn position_on_cursor_monitor(window_size: (f32, f32)) -> Option<(f32, f32)> {
    unsafe {
        let mut cursor = POINT::default();
        GetCursorPos(&mut cursor).ok()?;
        position_on_monitor(
            MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST),
            window_size,
        )
    }
}

/// Test hook: returns the top-left position within the work area of whichever display
/// the given point sits on (`ISSEN_DEBUG_FORCE_MONITOR_POINT`). Doesn't depend on
/// `GetCursorPos`, so a screen-capture harness can target an arbitrary display at a
/// predictable coordinate.
pub fn position_on_point_monitor(point: (i32, i32), window_size: (f32, f32)) -> Option<(f32, f32)> {
    unsafe {
        let p = POINT {
            x: point.0,
            y: point.1,
        };
        position_on_monitor(MonitorFromPoint(p, MONITOR_DEFAULTTONEAREST), window_size)
    }
}

/// Returns the top-left position within the primary display's work area
/// (`DisplayTarget::Primary`). Passing `MonitorFromPoint` an arbitrary point (the
/// origin, here) together with `MONITOR_DEFAULTTOPRIMARY` always returns the primary
/// display, regardless of which display that point actually falls on.
pub fn position_on_primary_monitor(window_size: (f32, f32)) -> Option<(f32, f32)> {
    unsafe {
        let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        position_on_monitor(monitor, window_size)
    }
}

/// Returns the top-left position within the work area of the display holding whatever
/// window was in the foreground just before this call (`DisplayTarget::FocusedWindow`).
/// The caller (`app.rs::set_visible`) must call this right after sending the
/// `ViewportCommand` that will make the main window visible, before the OS processes
/// it — `ViewportCommand`s are only queued and don't steal focus synchronously, so at
/// this point `GetForegroundWindow` still returns whatever window the user was last on,
/// not Issen's own.
pub fn position_on_foreground_window_monitor(window_size: (f32, f32)) -> Option<(f32, f32)> {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_invalid() {
            return None;
        }
        position_on_monitor(
            MonitorFromWindow(hwnd, MONITOR_DEFAULTTOPRIMARY),
            window_size,
        )
    }
}

fn position_on_monitor(monitor: HMONITOR, window_size: (f32, f32)) -> Option<(f32, f32)> {
    unsafe {
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return None;
        }
        Some(centered_position(info.rcWork, window_size))
    }
}

fn centered_position(work: RECT, window_size: (f32, f32)) -> (f32, f32) {
    let work_width = (work.right - work.left) as f32;
    let work_height = (work.bottom - work.top) as f32;

    let x = work.left as f32 + (work_width - window_size.0) / 2.0;
    let y = work.top as f32 + (work_height - window_size.1) / 3.0;
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centers_horizontally_and_sits_in_upper_third_vertically() {
        let work = RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };
        let (x, y) = centered_position(work, (640.0, 60.0));
        assert_eq!(x, 640.0);
        assert_eq!(y, 340.0);
    }

    #[test]
    fn accounts_for_a_non_primary_monitors_offset() {
        // A secondary display sitting to the right of the primary one.
        let work = RECT {
            left: 1920,
            top: 0,
            right: 1920 + 1280,
            bottom: 1024,
        };
        let (x, y) = centered_position(work, (640.0, 60.0));
        assert_eq!(x, 1920.0 + 320.0);
        assert_eq!(y, (1024.0 - 60.0) / 3.0);
    }
}
