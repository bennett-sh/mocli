//! Low-level keyboard and mouse hooks on a dedicated thread.
//!
//! The popup never takes focus, so while it is open the keyboard hook diverts
//! keys to it. Hook callbacks must return quickly or Windows drops them. Key
//! contents are never logged here.

use std::{
    cell::RefCell,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicIsize, Ordering},
        mpsc,
    },
    thread,
};

use anyhow::{Context as _, Result};
use async_channel::Sender;
use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Accessibility::{HWINEVENTHOOK, SetWinEventHook},
        Input::KeyboardAndMouse::{GetAsyncKeyState, GetKeyState},
        WindowsAndMessaging::{
            CallNextHookEx, DispatchMessageW, EVENT_SYSTEM_FOREGROUND, GetForegroundWindow,
            GetMessageW, HC_ACTION, KBDLLHOOKSTRUCT, MSG, MSLLHOOKSTRUCT, SetWindowsHookExW,
            WH_KEYBOARD_LL, WH_MOUSE_LL, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
            WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_RBUTTONDOWN, WM_SYSKEYDOWN,
            WM_SYSKEYUP, WM_XBUTTONDOWN,
        },
    },
};

use super::{
    Event,
    hotkey::{Hotkey, Mods},
    input::{self, OWN_INPUT},
    keyboard::{self, KeyFilter, Verdict},
};

static CAPTURING: AtomicBool = AtomicBool::new(false);
/// The window that had focus when capture began.
static TARGET: AtomicIsize = AtomicIsize::new(0);
static POPUP_RECT: Mutex<Option<RECT>> = Mutex::new(None);

struct State {
    filter: KeyFilter,
    events: Sender<Event>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Starts or stops diverting the keyboard to the popup.
pub fn set_capture(capture: bool) {
    if capture && !CAPTURING.load(Ordering::SeqCst) {
        // SAFETY: plain query.
        TARGET.store(
            unsafe { GetForegroundWindow() }.0 as isize,
            Ordering::SeqCst,
        );
    }
    CAPTURING.store(capture, Ordering::SeqCst);
}

/// Screen area of the popup; clicks outside it close the popup.
pub fn set_popup_rect(rect: Option<RECT>) {
    if let Ok(mut current) = POPUP_RECT.lock() {
        *current = rect;
    }
}

/// Installs the hooks and returns once they are active.
pub fn spawn(hotkey: Hotkey, events: Sender<Event>) -> Result<()> {
    let (ready_tx, ready_rx) = mpsc::channel();

    thread::Builder::new()
        .name("input-hooks".into())
        .spawn(move || {
            STATE.set(Some(State {
                filter: KeyFilter::new(hotkey),
                events,
            }));
            let installed = install();
            let ok = installed.is_ok();
            let _ = ready_tx.send(installed);
            if ok {
                pump_messages();
            }
        })?;

    ready_rx.recv().context("the hook thread exited early")?
}

fn install() -> Result<()> {
    // SAFETY: the callbacks are `'static` functions, and this thread pumps
    // messages for as long as the hooks exist. Windows removes them on exit.
    unsafe {
        let module = GetModuleHandleW(None)?;
        SetWindowsHookExW(WH_KEYBOARD_LL, Some(on_key), Some(module.into()), 0)
            .context("failed to install the keyboard hook")?;
        SetWindowsHookExW(WH_MOUSE_LL, Some(on_mouse), Some(module.into()), 0)
            .context("failed to install the mouse hook")?;
        let foreground = SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(on_foreground),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        );
        anyhow::ensure!(
            !foreground.is_invalid(),
            "failed to watch the foreground window"
        );
    }
    Ok(())
}

fn pump_messages() {
    let mut message = MSG::default();
    // SAFETY: `message` is a valid out pointer.
    while unsafe { GetMessageW(&mut message, None, 0, 0) }.0 > 0 {
        unsafe { DispatchMessageW(&message) };
    }
}

fn send(event: Event) {
    STATE.with_borrow(|state| {
        if let Some(state) = state {
            // The channel is unbounded, so this only fails during shutdown.
            let _ = state.events.try_send(event);
        }
    });
}

fn dismiss() {
    if CAPTURING.swap(false, Ordering::SeqCst) {
        send(Event::Hide);
    }
}

fn is_down(vk: i32) -> bool {
    // SAFETY: plain query.
    unsafe { GetAsyncKeyState(vk) < 0 }
}

fn held_modifiers() -> Mods {
    Mods {
        shift: is_down(0x10),
        ctrl: is_down(0x11),
        alt: is_down(0x12),
        win: is_down(0x5B) || is_down(0x5C),
    }
}

fn caps_lock_on() -> bool {
    // SAFETY: plain queries. The first call refreshes this thread's key state,
    // which is otherwise stale because it never receives keyboard messages.
    unsafe {
        GetKeyState(0);
        GetKeyState(0x14) & 1 != 0
    }
}

unsafe extern "system" fn on_key(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        // SAFETY: for `HC_ACTION`, `lparam` points to this struct.
        let key = unsafe { *(lparam.0 as *const KBDLLHOOKSTRUCT) };
        if key.dwExtraInfo != OWN_INPUT && handle_key(wparam.0 as u32, key) {
            return LRESULT(1);
        }
    }
    // SAFETY: forwards the arguments unchanged.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// Returns whether the key must be withheld from the focused app.
fn handle_key(message: u32, key: KBDLLHOOKSTRUCT) -> bool {
    let vk = key.vkCode as u16;
    let down = matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN);
    if !down && !matches!(message, WM_KEYUP | WM_SYSKEYUP) {
        return false;
    }

    let mods = held_modifiers();
    let capturing = CAPTURING.load(Ordering::SeqCst);
    let verdict = STATE.with_borrow_mut(|state| {
        state.as_mut().map_or(Verdict::Pass, |state| {
            if down {
                state.filter.key_down(vk, mods, capturing)
            } else {
                state.filter.key_up(vk)
            }
        })
    });

    match verdict {
        Verdict::Pass => return false,
        Verdict::Swallow => {}
        Verdict::Toggle => {
            // Flipping the flag here means the hotkey frees the keyboard even
            // if the UI thread is stuck.
            set_capture(!capturing);
            send(if capturing { Event::Hide } else { Event::Show });
        }
        Verdict::Forward => {
            if let Some(keystroke) = keyboard::keystroke(vk, key.scanCode, mods, caps_lock_on()) {
                send(Event::Key(keystroke));
            }
        }
    }

    if down && (mods.alt || mods.win) {
        input::tap_dummy_key();
    }
    true
}

unsafe extern "system" fn on_mouse(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let pressed = matches!(
        wparam.0 as u32,
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN
    );
    if code == HC_ACTION as i32 && pressed && CAPTURING.load(Ordering::SeqCst) {
        // SAFETY: for `HC_ACTION`, `lparam` points to this struct.
        let point = unsafe { (*(lparam.0 as *const MSLLHOOKSTRUCT)).pt };
        let inside = POPUP_RECT
            .lock()
            .ok()
            .and_then(|rect| *rect)
            .is_some_and(|r| {
                (r.left..r.right).contains(&point.x) && (r.top..r.bottom).contains(&point.y)
            });
        if !inside {
            dismiss();
        }
    }
    // SAFETY: forwards the arguments unchanged.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

unsafe extern "system" fn on_foreground(
    _: HWINEVENTHOOK,
    _: u32,
    window: HWND,
    _: i32,
    _: i32,
    _: u32,
    _: u32,
) {
    if window.0 as isize != TARGET.load(Ordering::SeqCst) {
        dismiss();
    }
}
