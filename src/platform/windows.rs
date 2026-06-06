use std::ptr::null_mut;
use windows_sys::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentProcessId;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub struct KeyboardGrab {
    hook: HHOOK,
}

pub struct WindowState {
    hwnd: HWND,
    style: isize,
    rect: RECT,
}

impl KeyboardGrab {
    pub fn install() -> Self {
        unsafe {
            let hook = SetWindowsHookExW(
                WH_KEYBOARD_LL,
                Some(low_level_keyboard_proc),
                GetModuleHandleW(null_mut()) as HINSTANCE,
                0,
            );

            if hook.is_null() {
                panic!("failed to install low-level keyboard hook");
            }

            Self { hook }
        }
    }
}

impl Drop for KeyboardGrab {
    fn drop(&mut self) {
        unsafe {
            UnhookWindowsHookEx(self.hook);
        }
    }
}

pub fn set_window_resizable(resizable: bool) {
    unsafe {
        let hwnd = foreground_window_for_current_process();
        if hwnd.is_null() {
            return;
        }

        let flags = (WS_MAXIMIZEBOX | WS_SIZEBOX) as isize;
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        let style = if resizable {
            style | flags
        } else {
            style & !flags
        };

        SetWindowLongPtrW(hwnd, GWL_STYLE, style);
        SetWindowPos(
            hwnd,
            HWND_TOP,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_FRAMECHANGED,
        );
    }
}

impl WindowState {
    pub fn fullscreen() -> Option<Self> {
        unsafe {
            let hwnd = foreground_window_for_current_process();
            if hwnd.is_null() {
                return None;
            }

            let mut rect = RECT::default();
            if GetWindowRect(hwnd, &mut rect) == 0 {
                return None;
            }

            let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                rcMonitor: RECT::default(),
                rcWork: RECT::default(),
                dwFlags: 0,
            };
            if GetMonitorInfoW(monitor, &mut info) == 0 {
                return None;
            }

            let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
            let decorations = (WS_CAPTION
                | WS_THICKFRAME
                | WS_SIZEBOX
                | WS_SYSMENU
                | WS_MINIMIZEBOX
                | WS_MAXIMIZEBOX) as isize;
            SetWindowLongPtrW(hwnd, GWL_STYLE, style & !decorations);

            let monitor = info.rcMonitor;
            SetWindowPos(
                hwnd,
                HWND_TOP,
                monitor.left,
                monitor.top,
                monitor.right - monitor.left,
                monitor.bottom - monitor.top,
                SWP_FRAMECHANGED | SWP_NOOWNERZORDER,
            );

            Some(Self { hwnd, style, rect })
        }
    }
}

impl Drop for WindowState {
    fn drop(&mut self) {
        unsafe {
            SetWindowLongPtrW(self.hwnd, GWL_STYLE, self.style);
            SetWindowPos(
                self.hwnd,
                HWND_TOP,
                self.rect.left,
                self.rect.top,
                self.rect.right - self.rect.left,
                self.rect.bottom - self.rect.top,
                SWP_FRAMECHANGED | SWP_NOOWNERZORDER,
            );
        }
    }
}

unsafe extern "system" fn low_level_keyboard_proc(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if code >= 0 {
        let hwnd = foreground_window_for_current_process();
        if !hwnd.is_null() && repost_key_message(hwnd, wparam, lparam) {
            return 1;
        }
    }

    CallNextHookEx(null_mut(), code, wparam, lparam)
}

unsafe fn foreground_window_for_current_process() -> HWND {
    let hwnd = GetForegroundWindow();
    if hwnd.is_null() {
        return null_mut();
    }

    let mut pid = 0;
    GetWindowThreadProcessId(hwnd, &mut pid);
    if pid == GetCurrentProcessId() {
        hwnd
    } else {
        null_mut()
    }
}

unsafe fn repost_key_message(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) -> bool {
    let msg = match wparam as u32 {
        WM_KEYDOWN | WM_SYSKEYDOWN => WM_KEYDOWN,
        WM_KEYUP | WM_SYSKEYUP => WM_KEYUP,
        _ => return false,
    };

    let key = *(lparam as *const KBDLLHOOKSTRUCT);
    let lparam = make_key_lparam(&key, msg);
    PostMessageW(hwnd, msg, key.vkCode as WPARAM, lparam) != 0
}

fn make_key_lparam(key: &KBDLLHOOKSTRUCT, msg: u32) -> LPARAM {
    let repeat_count = 1;
    let scan_code = (key.scanCode & 0xff) << 16;
    let extended = if key.flags & LLKHF_EXTENDED != 0 {
        1 << 24
    } else {
        0
    };
    let transition = if matches!(msg, WM_KEYUP | WM_SYSKEYUP) {
        (1 << 30) | (1 << 31)
    } else {
        0
    };

    (repeat_count | scan_code | extended | transition) as LPARAM
}
