//! Win32 adjustments to the popup window. By default it never takes focus.
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
        Gdi::{
            GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromRect,
            MonitorFromWindow,
        },
    },
    UI::{
        HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{
            GWL_EXSTYLE, GWL_STYLE, GetForegroundWindow, GetWindowLongPtrW, GetWindowRect,
            HWND_TOPMOST, MA_NOACTIVATE, PostMessageW, SW_HIDE, SWP_FRAMECHANGED, SWP_NOACTIVATE,
            SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW, SetForegroundWindow,
            SetWindowLongPtrW, SetWindowPos, ShowWindow, WA_ACTIVE, WA_INACTIVE, WM_ACTIVATE,
            WM_MOUSEACTIVATE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
        },
    },
};

use super::{input, placement};

/// Space between the popup and the caret, in logical pixels.
const ANCHOR_GAP: f32 = 8.0;

#[derive(Clone, Copy)]
pub struct PopupWindow {
    window: HWND,
    takes_focus: bool,
}

impl PopupWindow {
    pub fn of(window: &Window, takes_focus: bool) -> Result<Self> {
        // Fully qualified: `Window` has an inherent `window_handle` of its own.
        let handle = raw_window_handle::HasWindowHandle::window_handle(window)
            .map_err(|error| anyhow::anyhow!("no window handle: {error}"))?;
        match handle.as_raw() {
            RawWindowHandle::Win32(handle) => Ok(Self {
                window: HWND(handle.hwnd.get() as *mut _),
                takes_focus,
            }),
            other => anyhow::bail!("unexpected window handle {other:?}"),
        }
    }

    pub fn handle(self) -> HWND {
        self.window
    }

    /// Switches between taking focus and leaving it in the app. The popup must
    /// be hidden.
    pub fn set_takes_focus(&mut self, takes_focus: bool) -> Result<()> {
        self.takes_focus = takes_focus;
        self.prepare()
    }

    pub fn prepare(self) -> Result<()> {
        // SAFETY: the popup is a live window owned by the calling thread.
        unsafe {
            let mut style = GetWindowLongPtrW(self.window, GWL_EXSTYLE)
                | (WS_EX_TOOLWINDOW | WS_EX_TOPMOST).0 as isize;
            if self.takes_focus {
                style &= !(WS_EX_NOACTIVATE.0 as isize);
            } else {
                style |= WS_EX_NOACTIVATE.0 as isize;
            }
            SetWindowLongPtrW(self.window, GWL_EXSTYLE, style);
            // GPUI creates an overlapped window, whose invisible resize frame
            // would leave a gap around the content.
            SetWindowLongPtrW(self.window, GWL_STYLE, WS_POPUP.0 as isize);
            SetWindowPos(
                self.window,
                None,
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            )?;

            // GPUI answers `MA_ACTIVATE` to mouse clicks, overriding the style.
            if self.takes_focus {
                // Fails if it was never added, which is fine.
                let _ = RemoveWindowSubclass(self.window, Some(refuse_activation), 1);
            } else {
                SetWindowSubclass(self.window, Some(refuse_activation), 1, 0)
                    .ok()
                    .context("failed to subclass the popup")?;
            }

            // Rounded corners need Windows 11; older versions just stay square.
            let corners = DWMWCP_ROUND;
            let _ = DwmSetWindowAttribute(
                self.window,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                (&raw const corners).cast(),
                size_of_val(&corners) as u32,
            );
        }
        Ok(())
    }

    /// Shows the popup next to `anchor`, or in the middle of the focused app's
    /// monitor, and returns its screen rectangle. `anchor` is in physical
    /// pixels, `width` and `height` are logical.
    pub fn show(self, anchor: Option<RECT>, width: f32, height: f32) -> Result<RECT> {
        // SAFETY: all out pointers are valid locals.
        unsafe {
            let monitor = match &anchor {
                Some(anchor) => MonitorFromRect(anchor, MONITOR_DEFAULTTONEAREST),
                None => MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTONEAREST),
            };
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
            let scaled = |value: f32| (value * scale).round() as i32;
            let width = scaled(width).min(work.right - work.left);
            let height = scaled(height).min(work.bottom - work.top);
            let at = match anchor {
                Some(anchor) => placement::beside(anchor, work, width, height, scaled(ANCHOR_GAP)),
                None => placement::centered(work, width, height),
            };

            // Moving to a monitor with another DPI makes GPUI resize the
            // window, so apply the geometry a second time.
            for _ in 0..2 {
                SetWindowPos(
                    self.window,
                    Some(HWND_TOPMOST),
                    at.x,
                    at.y,
                    width,
                    height,
                    SWP_NOACTIVATE | SWP_SHOWWINDOW,
                )?;
            }
            if self.takes_focus {
                // Windows only lets the process that sent the latest input
                // take the foreground.
                input::tap_dummy_key();
                if !SetForegroundWindow(self.window).as_bool() {
                    tracing::warn!("Windows did not let the popup take focus");
                }
            } else {
                // Tells GPUI the window is active, which makes it draw the
                // caret. Windows' own focus does not change.
                self.post_activation(WA_ACTIVE)?;
            }

            let mut rect = RECT::default();
            GetWindowRect(self.window, &mut rect)?;
            Ok(rect)
        }
    }

    /// Hides the popup. If it holds focus, `restore` gets it back.
    pub fn hide(self, restore: Option<HWND>) -> Result<()> {
        // SAFETY: the popup is a live window; a stale `restore` is rejected.
        unsafe {
            if let Some(restore) = restore
                && self.takes_focus
                && GetForegroundWindow() == self.window
                && !SetForegroundWindow(restore).as_bool()
            {
                tracing::warn!("could not give focus back");
            }
            // The return value is the previous visibility, not an error.
            let _ = ShowWindow(self.window, SW_HIDE);
            if !self.takes_focus {
                self.post_activation(WA_INACTIVE)?;
            }
        }
        Ok(())
    }

    unsafe fn post_activation(self, state: u32) -> Result<()> {
        // SAFETY: `WM_ACTIVATE` carries no pointers here.
        unsafe {
            PostMessageW(
                Some(self.window),
                WM_ACTIVATE,
                WPARAM(state as usize),
                LPARAM(0),
            )?;
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
