//! Win32 adjustments that keep the popup from ever taking focus.
//!
//! These calls send window messages synchronously, so run them while GPUI's
//! `App` is not borrowed.

use anyhow::{Context as _, Result};
use gpui_kit::Window;
use raw_window_handle::RawWindowHandle;
use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::{
        Dwm::{DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute},
        Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow},
    },
    UI::{
        HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
        Shell::{DefSubclassProc, SetWindowSubclass},
        WindowsAndMessaging::{
            GWL_EXSTYLE, GWL_STYLE, GetForegroundWindow, GetWindowLongPtrW, GetWindowRect,
            HWND_TOPMOST, MA_NOACTIVATE, PostMessageW, SW_HIDE, SWP_FRAMECHANGED, SWP_NOACTIVATE,
            SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW, SetWindowLongPtrW, SetWindowPos,
            ShowWindow, WA_ACTIVE, WA_INACTIVE, WM_ACTIVATE, WM_MOUSEACTIVATE, WS_EX_NOACTIVATE,
            WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
        },
    },
};

#[derive(Clone, Copy)]
pub struct PopupWindow(HWND);

impl PopupWindow {
    pub fn of(window: &Window) -> Result<Self> {
        // Fully qualified: `Window` has an inherent `window_handle` of its own.
        let handle = raw_window_handle::HasWindowHandle::window_handle(window)
            .map_err(|error| anyhow::anyhow!("no window handle: {error}"))?;
        match handle.as_raw() {
            RawWindowHandle::Win32(handle) => Ok(Self(HWND(handle.hwnd.get() as *mut _))),
            other => anyhow::bail!("unexpected window handle {other:?}"),
        }
    }

    pub fn handle(self) -> HWND {
        self.0
    }

    pub fn prepare(self) -> Result<()> {
        // SAFETY: `self.0` is a live window owned by the calling thread.
        unsafe {
            let style = GetWindowLongPtrW(self.0, GWL_EXSTYLE);
            let extra = (WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST).0 as isize;
            SetWindowLongPtrW(self.0, GWL_EXSTYLE, style | extra);
            // GPUI creates an overlapped window, whose invisible resize frame
            // would leave a gap around the content.
            SetWindowLongPtrW(self.0, GWL_STYLE, WS_POPUP.0 as isize);
            SetWindowPos(
                self.0,
                None,
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            )?;

            // GPUI answers `MA_ACTIVATE` to mouse clicks, overriding the style.
            SetWindowSubclass(self.0, Some(refuse_activation), 1, 0)
                .ok()
                .context("failed to subclass the popup")?;

            // Rounded corners need Windows 11; older versions just stay square.
            let corners = DWMWCP_ROUND;
            let _ = DwmSetWindowAttribute(
                self.0,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                (&raw const corners).cast(),
                size_of_val(&corners) as u32,
            );
        }
        Ok(())
    }

    /// Shows the popup in the middle of the focused app's monitor and returns
    /// its screen rectangle. `width` and `height` are logical pixels.
    pub fn show_centered(self, width: f32, height: f32) -> Result<RECT> {
        // SAFETY: all out pointers are valid locals.
        unsafe {
            let monitor = MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            GetMonitorInfoW(monitor, &mut info)
                .ok()
                .context("failed to query the monitor")?;
            let (mut dpi, mut dpi_y) = (96, 96);
            GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut dpi_y)?;

            let work = info.rcWork;
            let scale = dpi as f32 / 96.0;
            let width = ((width * scale).round() as i32).min(work.right - work.left);
            let height = ((height * scale).round() as i32).min(work.bottom - work.top);
            let x = work.left + (work.right - work.left - width) / 2;
            let y = work.top + (work.bottom - work.top - height) / 2;

            // Moving to a monitor with another DPI makes GPUI resize the
            // window, so apply the geometry a second time.
            for _ in 0..2 {
                SetWindowPos(
                    self.0,
                    Some(HWND_TOPMOST),
                    x,
                    y,
                    width,
                    height,
                    SWP_NOACTIVATE | SWP_SHOWWINDOW,
                )?;
            }
            // Tells GPUI the window is active, which makes it draw the caret.
            // Windows' own focus does not change.
            self.post_activation(WA_ACTIVE)?;

            let mut rect = RECT::default();
            GetWindowRect(self.0, &mut rect)?;
            Ok(rect)
        }
    }

    pub fn hide(self) -> Result<()> {
        // SAFETY: `self.0` is a live window.
        unsafe {
            // The return value is the previous visibility, not an error.
            let _ = ShowWindow(self.0, SW_HIDE);
            self.post_activation(WA_INACTIVE)
        }
    }

    unsafe fn post_activation(self, state: u32) -> Result<()> {
        // SAFETY: `WM_ACTIVATE` carries no pointers here.
        unsafe {
            PostMessageW(Some(self.0), WM_ACTIVATE, WPARAM(state as usize), LPARAM(0))?;
        }
        Ok(())
    }
}

unsafe extern "system" fn refuse_activation(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    if message == WM_MOUSEACTIVATE {
        return LRESULT(MA_NOACTIVATE as isize);
    }
    // SAFETY: forwards the arguments unchanged.
    unsafe { DefSubclassProc(window, message, wparam, lparam) }
}
