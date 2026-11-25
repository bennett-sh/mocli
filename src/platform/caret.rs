//! Finds where the focused app is taking text, so the popup can open next to it.
//!
//! No single API covers every app, so these are tried in turn: the Win32 caret,
//! UI Automation's text ranges, the older MSAA caret, and finally the bounds of
//! the focused input.
//!
//! Querying another app can hang, so it happens on its own thread and the
//! caller only waits briefly.

use std::{
    ptr, slice,
    sync::mpsc::{self, Receiver, Sender, SyncSender},
    thread,
    time::Duration,
};

use anyhow::Result;
use windows::{
    Win32::{
        Foundation::{HWND, POINT, RECT},
        Graphics::Gdi::ClientToScreen,
        System::{
            Com::{
                CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
                SAFEARRAY,
            },
            Ole::{
                SafeArrayAccessData, SafeArrayDestroy, SafeArrayGetUBound, SafeArrayUnaccessData,
            },
            Variant::{VARIANT, VT_I4},
        },
        UI::{
            Accessibility::{
                AccessibleObjectFromWindow, CUIAutomation8, IAccessible, IUIAutomation,
                IUIAutomationElement, IUIAutomationTextPattern, IUIAutomationTextPattern2,
                IUIAutomationTextRange, TextPatternRangeEndpoint_Start, TextUnit_Character,
                UIA_ComboBoxControlTypeId, UIA_EditControlTypeId, UIA_TextPattern2Id,
                UIA_TextPatternId,
            },
            HiDpi::{
                DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow,
                GetWindowDpiAwarenessContext, LogicalToPhysicalPointForPerMonitorDPI,
                SetThreadDpiAwarenessContext,
            },
            WindowsAndMessaging::{
                GUITHREADINFO, GetGUIThreadInfo, GetWindowRect, GetWindowThreadProcessId,
                OBJID_CARET,
            },
        },
    },
    core::{BOOL, Interface as _},
};

const TIMEOUT: Duration = Duration::from_millis(250);
/// A focused element taller than this, in logical pixels, is a document or a
/// whole window rather than an input to sit next to.
const MAX_INPUT_HEIGHT: i32 = 120;

struct Request {
    window: isize,
    reply: SyncSender<Option<RECT>>,
}

pub struct Locator(Sender<Request>);

impl Locator {
    pub fn spawn() -> Result<Self> {
        let (requests, incoming) = mpsc::channel();
        thread::Builder::new()
            .name("caret".into())
            .spawn(move || serve(incoming))?;
        Ok(Self(requests))
    }

    /// The screen rectangle, in physical pixels, of the caret or input in
    /// `window`. `None` if there is none to be found in time.
    pub fn locate(&self, window: HWND) -> Option<RECT> {
        let (reply, response) = mpsc::sync_channel(1);
        self.0
            .send(Request {
                window: window.0 as isize,
                reply,
            })
            .ok()?;
        response.recv_timeout(TIMEOUT).ok().flatten()
    }
}

fn serve(requests: Receiver<Request>) {
    // SAFETY: called once on this thread, before any COM or window query.
    let automation: Option<IUIAutomation> = unsafe {
        SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        CoInitializeEx(None, COINIT_MULTITHREADED)
            .ok()
            .and_then(|()| CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER))
            .inspect_err(|error| tracing::warn!(%error, "UI Automation is unavailable"))
            .ok()
    };

    while let Ok(mut request) = requests.recv() {
        // Only the newest request still has someone waiting.
        while let Ok(newer) = requests.try_recv() {
            request = newer;
        }
        let window = HWND(request.window as *mut _);
        let found = find(automation.as_ref(), window);
        tracing::debug!(
            method = found.map_or("none", |(_, method)| method),
            "caret lookup"
        );
        // The caller may have stopped waiting.
        let _ = request.reply.try_send(found.map(|(rect, _)| rect));
    }
}

fn find(automation: Option<&IUIAutomation>, window: HWND) -> Option<(RECT, &'static str)> {
    let mut info = GUITHREADINFO {
        cbSize: size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: `info` is a valid out pointer with its size set.
    let focus = unsafe {
        let thread = GetWindowThreadProcessId(window, None);
        match GetGUIThreadInfo(thread, &mut info) {
            Ok(()) if !info.hwndFocus.is_invalid() => info.hwndFocus,
            _ => window,
        }
    };
    let plausible = |rect: Option<RECT>| rect.filter(|&rect| is_plausible(rect, window));

    if let Some(rect) = plausible(system_caret(&info)) {
        return Some((rect, "system caret"));
    }
    // SAFETY: COM is initialized on this thread.
    let element = automation.and_then(|a| unsafe { a.GetFocusedElement() }.ok());
    if let Some(rect) = plausible(element.as_ref().and_then(text_caret)) {
        return Some((rect, "text range"));
    }
    if let Some(rect) = plausible(accessible_caret(focus)) {
        return Some((rect, "accessible caret"));
    }
    let rect = plausible(element.as_ref().and_then(|e| input_bounds(e, window)))?;
    Some((rect, "input bounds"))
}

/// Some apps report stale or made-up positions; believe only those inside
/// their own window.
fn is_plausible(rect: RECT, window: HWND) -> bool {
    let mut bounds = RECT::default();
    // SAFETY: `bounds` is a valid out pointer.
    if unsafe { GetWindowRect(window, &mut bounds) }.is_err() {
        return false;
    }
    let x = rect.left + (rect.right - rect.left) / 2;
    let y = rect.top + (rect.bottom - rect.top) / 2;
    rect.bottom > rect.top
        && (bounds.left..=bounds.right).contains(&x)
        && (bounds.top..=bounds.bottom).contains(&y)
}

/// The caret classic Win32 controls create.
fn system_caret(info: &GUITHREADINFO) -> Option<RECT> {
    let window = info.hwndCaret;
    let caret = info.rcCaret;
    if window.is_invalid() || caret.bottom <= caret.top {
        return None;
    }
    let top_left = on_screen(window, caret.left, caret.top)?;
    let bottom_right = on_screen(window, caret.right, caret.bottom)?;
    Some(RECT {
        left: top_left.x,
        top: top_left.y,
        right: bottom_right.x,
        bottom: bottom_right.y,
    })
}

/// Converts a point in `window`'s client area to physical screen pixels, also
/// for apps that Windows scales.
fn on_screen(window: HWND, x: i32, y: i32) -> Option<POINT> {
    let mut point = POINT { x, y };
    // SAFETY: the thread's DPI context is restored before returning.
    unsafe {
        let previous = SetThreadDpiAwarenessContext(GetWindowDpiAwarenessContext(window));
        let converted = ClientToScreen(window, &mut point);
        SetThreadDpiAwarenessContext(previous);
        if !converted.as_bool() {
            return None;
        }
        // Leaves the point alone if the app is already DPI aware.
        let _ = LogicalToPhysicalPointForPerMonitorDPI(Some(window), &mut point);
    }
    Some(point)
}

/// The caret as UI Automation describes it: browsers, Electron, WPF, WinUI.
fn text_caret(element: &IUIAutomationElement) -> Option<RECT> {
    // SAFETY: COM calls on interfaces obtained on this thread.
    unsafe {
        let mut active = BOOL::default();
        let range = element
            .GetCurrentPatternAs::<IUIAutomationTextPattern2>(UIA_TextPattern2Id)
            .and_then(|pattern| pattern.GetCaretRange(&mut active))
            .or_else(|_| {
                element
                    .GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
                    .and_then(|pattern| pattern.GetSelection()?.GetElement(0))
            })
            .ok()?;
        if let Some(rect) = range_bounds(&range) {
            return Some(rect);
        }
        // An empty range often has no rectangle: measure a neighbour instead.
        let next = range.Clone().ok()?;
        next.ExpandToEnclosingUnit(TextUnit_Character).ok()?;
        if let Some(rect) = range_bounds(&next) {
            return Some(RECT {
                right: rect.left,
                ..rect
            });
        }
        let previous = range.Clone().ok()?;
        previous
            .MoveEndpointByUnit(TextPatternRangeEndpoint_Start, TextUnit_Character, -1)
            .ok()?;
        range_bounds(&previous).map(|rect| RECT {
            left: rect.right,
            ..rect
        })
    }
}

/// The last rectangle of a text range, which is where a selection ends.
fn range_bounds(range: &IUIAutomationTextRange) -> Option<RECT> {
    // SAFETY: the array holds doubles and is destroyed exactly once.
    unsafe {
        let array = range.GetBoundingRectangles().ok()?;
        if array.is_null() {
            return None;
        }
        let rect = last_rect(array);
        let _ = SafeArrayDestroy(array);
        rect
    }
}

/// # Safety
///
/// `array` must be a one-dimensional, zero-based array of `f64`.
unsafe fn last_rect(array: *mut SAFEARRAY) -> Option<RECT> {
    // SAFETY: guaranteed by the caller; access is released before returning.
    unsafe {
        let length = usize::try_from(SafeArrayGetUBound(array, 1).ok()? + 1).ok()?;
        if length < 4 {
            return None;
        }
        let mut data = ptr::null_mut();
        SafeArrayAccessData(array, &mut data).ok()?;
        let values = slice::from_raw_parts(data.cast::<f64>(), length);
        let [x, y, width, height] = *values.last_chunk::<4>()?;
        let _ = SafeArrayUnaccessData(array);
        (height > 0.0).then_some(RECT {
            left: x as i32,
            top: y as i32,
            right: (x + width) as i32,
            bottom: (y + height) as i32,
        })
    }
}

/// The caret as the older accessibility API reports it.
fn accessible_caret(window: HWND) -> Option<RECT> {
    // SAFETY: on success the pointer is an `IAccessible` we own.
    unsafe {
        let mut pointer = ptr::null_mut();
        AccessibleObjectFromWindow(
            window,
            OBJID_CARET.0 as u32,
            &IAccessible::IID,
            &mut pointer,
        )
        .ok()?;
        if pointer.is_null() {
            return None;
        }
        let accessible = IAccessible::from_raw(pointer);
        // `CHILDID_SELF`: the integer zero.
        let mut itself = VARIANT::default();
        (*itself.Anonymous.Anonymous).vt = VT_I4;
        let (mut left, mut top, mut width, mut height) = (0, 0, 0, 0);
        accessible
            .accLocation(&mut left, &mut top, &mut width, &mut height, &itself)
            .ok()?;
        (height > 0).then(|| RECT {
            left,
            top,
            right: left + width,
            bottom: top + height,
        })
    }
}

/// The focused element itself, if it looks like an input.
fn input_bounds(element: &IUIAutomationElement, window: HWND) -> Option<RECT> {
    // SAFETY: COM calls on an interface obtained on this thread.
    unsafe {
        let rect = element.CurrentBoundingRectangle().ok()?;
        let kind = element.CurrentControlType().ok()?;
        let limit = MAX_INPUT_HEIGHT * GetDpiForWindow(window).max(96) as i32 / 96;
        let input = kind == UIA_EditControlTypeId || kind == UIA_ComboBoxControlTypeId;
        (input && rect.bottom - rect.top <= limit).then_some(rect)
    }
}
