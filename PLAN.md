# mocli — Clipboard History for Windows (gpui-kit)

A Raycast-style clipboard history for Windows 11, built in Rust on `gpui-kit`
(GPUI + gpui-component). Fluent/Mica look, keyboard-first, opens without
stealing focus from the text field you're typing in.

---

## 1. Goals

| # | Requirement | Design answer |
|---|---|---|
| 1 | Global shortcut (default `Win+V`, fallback `Alt+V`) | Low-level keyboard hook (`WH_KEYBOARD_LL`) so `Win+V` can be swallowed before Explorer; `RegisterHotKey` for non-Win combos |
| 2 | Popup centered **or** at the caret, **without stealing focus** | `WindowKind::PopUp`, `focus: false`, `WS_EX_NOACTIVATE \| WS_EX_TOOLWINDOW \| WS_EX_TOPMOST`; keys routed to us via the LL hook while open |
| 3 | Type to search, ↑/↓ to navigate | Hook forwards keystrokes into the in-app search `InputState`; arrows move selection |
| 4 | `Enter` paste into focused input | Write entry to clipboard → `SendInput(Ctrl+V)` into the still-focused target → optionally restore previous clipboard |
| 5 | `Ctrl+Enter` copy only | Write to clipboard, close |
| 6 | `Ctrl+K` actions panel | Popover menu: Paste, Copy, Paste as plain text, Pin, Open/Reveal, Copy path, Delete, Delete all from app, … + **Settings** at bottom |
| 7 | Images | Capture `CF_DIB/CF_DIBV5/PNG`, store as PNG + thumbnail; image search via Windows OCR |
| 8 | Rich previews + metadata | Per-kind preview renderers (text, code, link w/ OG image, color, image, svg, file/exe icon, file list) + "Information" table |

Non-goals (v1): sync across devices, macOS/Linux, plugin API.

---

## 2. Tech stack

| Concern | Crate / API |
|---|---|
| UI | `gpui-kit` (= `gpui`, `gpui-component`, `gpui-kit-assets`) — pin to **0.11** (0.10 is the one cached locally; verify API diffs on first build) |
| Win32 | `windows` crate (`Win32_UI_WindowsAndMessaging`, `Win32_System_DataExchange`, `Win32_UI_Input_KeyboardAndMouse`, `Win32_UI_Shell`, `Win32_Graphics_Dwm`, `Win32_UI_Accessibility`, `Media_Ocr`, `Graphics_Imaging`) |
| Clipboard I/O | `clipboard-win` for formats + raw Win32 for listener / exclusion formats |
| Storage | `rusqlite` (bundled, FTS5) + blob dir for images |
| Images | `image` (decode DIB/PNG, thumbnails), `resvg`/`usvg` (SVG raster for thumbnails; gpui renders SVG natively for previews) |
| Links | `reqwest` + `scraper` for title/favicon/OG image (opt-in, off for private mode) |
| Fuzzy search | `nucleo-matcher` for ranking on top of FTS5 prefilter |
| Config | `serde` + `toml`, `notify` for hot reload, `directories` for paths |
| Autostart | `HKCU\...\Run` key |
| Tray | `tray-icon` (or raw `Shell_NotifyIcon`) |
| Logging | `tracing` + `tracing-appender` |

---

## 3. Architecture

```
┌──────────────────────────── mocli.exe (single instance) ───────────────────────────┐
│                                                                                     │
│  platform/ (Win32 thread, message-only HWND)        app/ (GPUI main thread)         │
│  ├─ clipboard_watcher  ──WM_CLIPBOARDUPDATE──►  ingest ──► store (SQLite + blobs)   │
│  ├─ hotkey (LL hook)   ──Toggle──────────────►  PopupController                     │
│  ├─ key_router (LL hook, active while open) ──►   ├─ ClipboardView (gpui)           │
│  ├─ caret_locator  ◄── where to show? ────────    │   ├─ SearchBar + TypeFilter     │
│  ├─ paster (SendInput)  ◄── Paste(entry) ─────    │   ├─ HistoryList (virtual)      │
│  ├─ source_app (owner HWND → exe, icon)           │   ├─ PreviewPane + InfoTable    │
│  └─ tray                                          │   └─ Footer + ActionsPanel      │
│                                                   └─ SettingsWindow (normal window)│
└─────────────────────────────────────────────────────────────────────────────────────┘
```

Win32 side runs its own thread with a message loop (hooks and the clipboard
listener need one); it talks to GPUI over a `flume`/`async-channel` channel
polled from `cx.spawn`. GPUI side never blocks on Win32.

### Module layout

```
src/
  main.rs                 // single-instance mutex, init tracing, start platform thread, run gpui app
  app.rs                  // global App state, actions!(), keybindings
  config/ {mod, schema, watch}.rs
  store/  {mod, schema.sql, entry, search, retention, blobs}.rs
  ingest/ {mod, classify, color, link, file, image, ocr}.rs
  platform/windows/
          {mod, clipboard_watcher, clipboard_write, hotkey, key_router,
           caret, paste, source_app, icons, window_style, backdrop, tray, autostart}.rs
  ui/
    popup.rs              // root view of the popup window
    search_bar.rs
    type_filter.rs        // Select/Popover "All Types ▾"
    history_list.rs       // ListDelegate impl, sections Today / Yesterday / This Week / …
    row.rs                // icon + title + pin/kbd hints
    preview/ {mod, text, code, link, color, image, svg, file, files}.rs
    info_table.rs
    actions_panel.rs      // Ctrl+K
    footer.rs
    settings/ {mod, general, appearance, shortcuts, privacy, storage, about}.rs
    theme.rs              // Fluent tokens → gpui-component Theme
```

---

## 4. Data model

The schema lives in `src/store/schema.sql` (versioned through `user_version`):

- `entries`: one row per distinct content, keyed by a blake3 hash. Copying the
  same content again bumps `last_copied` and `copy_count`. Kinds: `text`, `link`,
  `email`, `color`, `image`, `files`.
- `formats`: HTML and RTF payloads, restored when pasting.
- `entries_fts`: trigram FTS5 index over title, text and source name.
- Images are `blobs/<hash>.png` plus `<hash>.thumb.png`, next to `history.db` in
  `%LOCALAPPDATA%\mocli` (`MOCLI_DATA_DIR` overrides it). Nothing is encrypted.

Columns for link metadata (M4) and OCR text (M6) arrive with migrations.

Classification (`src/ingest/classify.rs`): files win; an image wins unless it
comes with real text (a lone URL or path doesn't count, so spreadsheet cells stay
text); text is then color, email, link or plain. Named CSS colors are not treated
as colors, because "red" or "tan" are usually words. `code` and `svg` kinds are
added with their previews in M4.

---

## 5. The hard parts (Windows specifics)

### 5.1 Not stealing focus
- Create popup with `WindowOptions { kind: WindowKind::PopUp, focus: false, show: false, is_movable: false, titlebar: None, window_background: MicaBackdrop|Blurred, .. }`.
- After creation, patch the HWND: add `WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST`, show with `SW_SHOWNOACTIVATE`. Rounded corners via `DwmSetWindowAttribute(DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND)`.
- Because we're never the foreground window, gpui won't receive keyboard input. **Key router**: while the popup is open, the LL hook intercepts key-downs, swallows them (returns 1) and forwards `(vk, scancode, modifiers)` to GPUI, where they're dispatched as gpui `KeyDownEvent`s / actions into the popup's focused `InputState`. Use `ToUnicodeEx` with the target thread's keyboard layout (`GetKeyboardLayout(target_tid)`) so typed characters match the user's layout (incl. dead keys / AltGr).
- Mouse clicks: `WS_EX_NOACTIVATE` still delivers clicks, so list clicks/scroll work; handle `WM_MOUSEACTIVATE → MA_NOACTIVATE`.
- Close on: `Esc`, hotkey again, click outside (LL mouse hook or `WM_ACTIVATEAPP` of the target via `SetWinEventHook(EVENT_SYSTEM_FOREGROUND)`), or foreground change.
- **Fallback mode** (setting "Focus popup"): normal activating window; remember `GetForegroundWindow()` + focused control (`GetGUIThreadInfo.hwndFocus`) and restore with `SetForegroundWindow` (+ `AttachThreadInput` dance) before pasting. Needed for elevated targets where the LL hook can't inject into them (UIPI) — detect and auto-fallback.

### 5.2 Placing near the input box
Priority chain in `platform/windows/caret.rs`:
1. `GetGUIThreadInfo(target_tid).rcCaret` (Win32 edits, many native apps) → `ClientToScreen`.
2. UI Automation: focused element → `TextPattern2.GetCaretRange` → `GetBoundingRectangles` (Chrome, Edge, Electron, WinUI, Office).
3. MSAA `AccessibleObjectFromWindow(OBJID_CARET)`.
4. Focused element's bounding rect (UIA `CurrentBoundingRectangle`) — place below it.
5. Mouse cursor position.

Then clamp to the monitor work area (`MonitorFromPoint` + `GetMonitorInfo`), flip above the caret if not enough room below, respect per-monitor DPI.
Placement setting: `centered | caret | cursor | last_position`, default `caret` with centered fallback.

### 5.3 The `Win+V` shortcut
Explorer owns `Win+V` (system clipboard history) — `RegisterHotKey` fails. The LL hook sees it first, so we swallow `V`-down while `LWin/RWin` is held, and also swallow the subsequent `Win`-up (send a dummy `VK_E8` to prevent the Start menu from opening). Settings offer: "Replace Windows clipboard history (Win+V)" (default **on**) and a link to turn off the system one (`HKCU\Software\Microsoft\Clipboard\EnableClipboardHistory = 0`) — only touched after the user confirms. `Alt+V` and arbitrary combos via the same hook or `RegisterHotKey`.

### 5.4 Pasting
1. Set clipboard (all stored formats; images as `CF_DIBV5` + `PNG`; files as `CF_HDROP`). Mark our own write with a private format `mocli.self` so the watcher ignores it (or re-promotes the entry to top — setting).
2. Hide popup (no activation change needed in no-focus mode).
3. `SendInput`: release any held modifiers (Ctrl from Ctrl+Enter etc.), then `Ctrl+V`. Configurable per-app paste key (`Shift+Insert` for terminals, e.g. `WindowsTerminal.exe`, `mintty.exe`).
4. Optional "restore previous clipboard after paste" (default off) with ~150 ms delay.

### 5.5 Source app & privacy
- Source: `GetClipboardOwner()` → `GetWindowThreadProcessId` → `QueryFullProcessImageNameW`; fall back to foreground window at copy time. Friendly name from `FileDescription` in the version resource. Icon via `SHGetFileInfoW`/`IShellItemImageFactory` → BGRA → gpui `ImageSource`, cached per exe.
- Honour password-manager hints: skip if clipboard has `ExcludeClipboardContentFromMonitorProcessing`, `CanIncludeInClipboardHistory = 0`, or `Clipboard Viewer Ignore`.
- Ignore list (exes) defaulting to common password managers (1Password, Bitwarden, KeePassXC); optional regex filters (credit cards, API key patterns) with "ask / skip / store".
- Private mode toggle (tray + action) pauses capture.

### 5.6 Previews

| Kind | Row icon | Preview | Info rows |
|---|---|---|---|
| text | doc icon | wrapped text (gpui-component `TextView`) | Source, Type, Characters, Words, Lines, Copied, Times copied |
| code | code icon | `highlighter` (feature `tree-sitter-*`) with detected language | + Language |
| link | site favicon | OG image card + title + domain (as in screenshot 1) | URL, Title, Domain |
| email | `@` | mailto card | Address |
| color | swatch | big swatch + HEX/RGB/HSL/OKLCH rows, each copyable; contrast vs white/black | Format, values |
| image | thumbnail | `img()` fitted, checkerboard for alpha | Dimensions, Size, Format, OCR text (collapsible) |
| svg (text or file) | rendered thumb | `svg()` rendering + toggle "Source" | Dimensions (viewBox), Size |
| file `.exe/.lnk/.dll` | extracted app icon | large icon + product name/version/publisher | Path, Size, Version, Modified |
| file image (`.png/.jpg/.webp`) | thumbnail | image | Path, Dimensions, Size |
| file other | shell icon | shell thumbnail (`IShellItemImageFactory`) | Path, Size, Modified |
| files (multiple) | stack icon | list with icons | Count, Total size |

Preview decoding runs on `cx.background_executor()`; LRU cache of decoded thumbnails.

---

## 6. UI (gpui-component first)

Popup layout mirrors the screenshots: **header** (back/clear button, search input, type filter) · **split body** (list ~40% | preview + info) · **footer** (app icon + "Clipboard History", primary action + `Kbd`, divider, "Actions" + `Ctrl K`).

| Piece | Component |
|---|---|
| Search field | `input::Input` / `InputState` (large, borderless, `appearance(false)`) |
| Type filter "All Types ▾" | `select::Select` with searchable dropdown (All, Text, Images, Files, Links, Emails, Colors, Code, Pinned) — matches screenshot 2 |
| History list | `list::List` + `ListDelegate` (virtualized, section headers "Pinned / Today / Yesterday / This Week / This Month / Older", keyboard selection built in) |
| Row | `ListItem` + `Icon`/`img`, `Kbd` for quick-select `Ctrl+1…9` |
| Preview text | `text::TextView` / `highlighter` |
| Info table | `description_list::DescriptionList` |
| Footer hints | `kbd::Kbd`, `button::Button` ghost, `separator::Separator` |
| Actions (`Ctrl+K`) | `command::Command` inside a `popover::Popover` anchored bottom-right — searchable, grouped, shortcut hints |
| Delete confirm | `dialog` (only for "Delete all") |
| Toasts ("Copied", "Deleted") | `notification` |
| Empty state | `empty::Empty` |
| Loading previews | `skeleton` / `spinner` |
| Settings window | `setting::Settings` (pages + groups + fields) in a normal window with `title_bar::TitleBar`; `switch`, `select`, `slider`, `color_picker`, `input`, `radio`, `tab`; custom shortcut-recorder field |
| Tooltips | `tooltip` |

### Theme
- `WindowBackgroundAppearance::MicaBackdrop` (Win11) → `Blurred` (acrylic) → opaque fallback. Setting: `mica | mica_alt | acrylic | solid`.
- Theme tokens tuned for translucency: background `transparent`/low alpha, list hover = `white/6%`, selected = `white/10%` + 8 px radius, 1 px border `white/8%`, Segoe UI Variable, 14/16/20 type ramp, Fluent System Icons (or Lucide from `gpui-kit-assets`).
- Follow system light/dark (`cx.window_appearance()`), accent from `DwmGetColorizationColor` (optional).
- Subtle open animation (fade + 4 px slide, 120 ms), honour "reduce motion".

---

## 7. Keyboard map (popup)

| Key | Action |
|---|---|
| any printable | type into search |
| `↑ ↓` / `Ctrl+P/N` | move selection |
| `PgUp/PgDn`, `Home/End` (when search empty) | jump |
| `Enter` | paste into focused input |
| `Ctrl+Enter` | copy to clipboard |
| `Shift+Enter` | paste as plain text |
| `Ctrl+K` | actions panel |
| `Ctrl+1…9` | paste n-th item |
| `Ctrl+P` (configurable) / `Ctrl+Shift+P` | pin / unpin |
| `Ctrl+Backspace` / `Del` (search empty) | delete entry |
| `Ctrl+F` / `Tab` | cycle type filter |
| `Ctrl+,` | open settings |
| `Esc` | clear search → close |

All bound via gpui `actions!` + `KeyBinding` in a `ClipboardPopup` key context; user-overridable in config.

### Actions panel contents (`Ctrl+K`)
Paste · Copy · Paste as Plain Text · Pin/Unpin · Edit & Paste (text) · Open (link/file) · Reveal in Explorer · Copy Path · Copy as HEX/RGB/HSL (colors) · Copy OCR text (images) · Save image as… · Delete · Delete all from *<app>* · Ignore *<app>* · — · Private mode · Clear history… · **Settings…** · Quit

---

## 8. Settings (opinionated defaults)

```toml
[general]
hotkey = "Win+V"            # also "Alt+V", any combo
replace_windows_history = true
launch_at_login = true
placement = "caret"         # caret | centered | cursor | last
focus_mode = "no_activate"  # no_activate | activate (fallback)
primary_action = "paste"    # paste | copy  (Enter vs Ctrl+Enter swap)
close_after_action = true
restore_clipboard_after_paste = false
move_to_top_on_reuse = true

[appearance]
theme = "system"            # system | light | dark
backdrop = "mica"           # mica | mica_alt | acrylic | solid
size = "medium"             # compact | medium | large (or width/height)
preview_pane = true
show_source_icon = true
animations = true

[history]
max_entries = 1000
max_age_days = 90           # 0 = forever; pinned exempt
max_image_mb = 25
capture = ["text","image","file","html"]
ocr_images = true
fetch_link_previews = true

[privacy]
ignore_apps = ["1Password.exe","Bitwarden.exe","KeePassXC.exe"]
respect_exclusion_formats = true
sensitive_patterns = "ask"  # ask | skip | store
private_mode = false

[paste]
per_app = { "WindowsTerminal.exe" = "Ctrl+Shift+V", "mintty.exe" = "Shift+Insert" }

[keys]                      # overrides for popup bindings
```

Settings pages: General · Appearance · Shortcuts · History · Privacy · Advanced (open data folder, export/import JSON, reset, logs) · About.

---

## 9. Milestones

| M | Scope | Done when |
|---|---|---|
| **0 Skeleton** | gpui-kit app, single-instance, tray, config load, tracing | Tray icon, quit works |
| **1 Capture & store** | clipboard watcher, classify text/link/email/color/image/file, SQLite + blobs, dedupe, retention, source app | Copying things fills the DB with correct kinds + source |
| **2 Popup (focus mode)** | Popup view with list/preview/footer, search (FTS + fuzzy), type filter, centered placement, Enter/Ctrl+Enter via activate+restore | Usable end-to-end in fallback mode |
| **3 No-focus popup** | `WS_EX_NOACTIVATE`, LL-hook key router with layout-correct text, Win+V takeover, caret placement chain, click-outside close | Caret stays blinking in Notepad/Chrome/VS Code while searching + pasting |
| **4 Previews** | color, image, svg, exe icon, shell thumbnails, link OG fetch, code highlight, info tables | Every kind in §5.6 renders |
| **5 Actions & settings** | Ctrl+K command popover, pin, delete, settings window, hot-reload, per-app paste keys | All §7/§8 items work |
| **6 Polish** | Mica/acrylic theme, animations, OCR search, privacy filters, autostart, installer (`cargo-wix` / MSIX), signed build | Daily-driver quality |

---

## 10. Spike results (2026-11-25, gpui-kit 0.11.0)

M0 is built, and the no-focus input approach **works**. Check it with
`scripts/e2e-popup.ps1`.

- `Window::dispatch_keystroke` drives key bindings and the `Input` component in a
  window that never has OS focus. The read-only-input fallback is not needed.
- GPUI answers `WM_MOUSEACTIVATE` with `MA_ACTIVATE`, overriding
  `WS_EX_NOACTIVATE`. The popup is subclassed to answer `MA_NOACTIVATE`.
- The input draws its caret and selection only in an "active" window. Posting
  `WM_ACTIVATE` to the popup sets GPUI's flag without moving OS focus.
- GPUI creates popups as overlapped windows with an invisible resize frame. The
  style is replaced with `WS_POPUP`.
- Mica turns solid on inactive windows, so the backdrop options are
  `acrylic | solid` (replaces the Mica default in §6 and §8).
- Type-ahead and AltGr work. Dead keys are handled by `ToUnicodeEx` but untested.
- The default hotkey is `Alt+V` for now. `Win+V` parses and should be swallowed
  by the same hook, but is untested.

The end-to-end script passes, and its screenshot confirms that acrylic blurs
what is behind the popup.

Still open: IME composition, elevated targets, tray menu clicks, other keyboard
layouts, mixed-DPI monitors.

## 11. M1 status (2026-11-25)

Capture and storage are built and checked by `scripts/e2e-capture.ps1`: kinds,
dedupe, HTML retention, images with transparency, file lists, source app, privacy
markers, ignored apps and persistence. Retention limits are unit tested.

Pulled forward from M6: exclusion formats and the ignored-apps list, so that
passwords never reach the database. Sensitive-text patterns remain in M6, private
mode in M5. Untested: the foreground-window fallback for ownerless copies, window
titles, and copies from elevated apps.

## 12. M2 status (2026-11-25)

The popup shows the real history and pastes from it, checked by
`scripts/e2e-popup.ps1`.

- Built directly on the no-focus popup. The planned "activate and restore" mode
  was not needed and moves to M3 as the fallback for IME and elevated targets.
- Keys: arrows, PgUp/PgDn, Home/End (empty search), Enter, Shift+Enter,
  Ctrl+Enter, Tab / Shift+Tab / Ctrl+F (type filter), Esc. A click selects, a
  double click pastes. Ctrl+K, pin, delete and Ctrl+1…9 are M5.
- Search: every word must occur (trigram FTS, `LIKE` for words under three
  characters), newest first; up to ten fuzzy title matches follow as "Similar".
- Pasting writes the clipboard with a `mocli.self` marker the watcher skips, then
  sends Ctrl+V. Per-app paste keys are M5.
- Previews are plain text or the image, plus the info table. Rich previews are M4.

Known gaps: opening from the tray leaves no focused input to paste into; pasting
is only verified with a Win32 text box; the light theme is unchecked.

## 13. M3 status (2026-11-25)

- Placement `caret` (default) tries the Win32 caret, UI Automation text ranges,
  the MSAA caret, then a small focused input, and otherwise centers. `centered`
  and `cursor` are the other settings; "last position" was dropped because the
  popup cannot be dragged.
- `Win+V` is the default shortcut and replaces the Windows clipboard history.
- `focus_mode = "activate"` is the fallback for input methods: the popup takes
  focus and hands it back before pasting. It is a manual setting; elevated apps
  are still unsupported.
- Pasting is skipped, leaving the entry on the clipboard, if focus moved.
- `scripts/e2e-placement.ps1` covers Win32, WPF, Edge and both focus modes.

## 14. M4–M6 and release status (2026-11-25)

Built as planned, with these differences:

- **Previews:** code is highlighted with syntect; SVG is its own kind. No
  checkerboard behind transparent images. Link previews are **off by default**
  (§2 and §8 disagreed): fetching a copied link contacts its server and can use
  up one-time links.
- **Actions:** bulk deletion is confirmed inside the panel instead of a dialog.
- **Settings:** `[keys]` needs a restart. The shortcut is a dropdown (any other
  combination via the config file), not a recorder. No accent color, JSON
  export/import or "keep open after action".
- **Privacy:** sensitive content is `skip | store`; "ask" was dropped. Start with
  Windows lives only in the registry, so the app and installer agree.
- **Mica** and a signed build are not done (the latter needs a certificate).
- **Packaging:** Inno Setup per-user installer and a portable zip
  (`scripts/package.ps1`); a `portable` file next to the executable keeps data
  there.
- **CI:** `dev` builds and packages; `main` releases the files built for the same
  source tree (see README, "Releases").

- **Overlays:** the type dropdown and actions panel show the window's acrylic.
  GPUI cannot blur inside a window (and has no native popups on Windows), so they
  are translucent and the content under them fades.

Verification: unit and headless GPUI tests cover capture, storage, previews,
keys, actions and settings, and all three end-to-end scripts pass against the
release build, including WPF, Edge and the "activate" focus mode. GPUI logs
"Invalid window handle" when the settings window closes; it is harmless.

## 15. Risks / open questions

- **Keyboard via LL hook** — must be fast (< ~1 ms, Windows drops slow hooks after `LowLevelHooksTimeout`); do minimal work in the hook, post to a channel. IME composition (CJK) won't work through the router → fallback to activate mode when the target uses an IME.
- **UIPI** — can't hook/inject into elevated apps unless mocli runs elevated; detect via `OpenProcessToken`+`TokenElevation` and warn/fallback.
- **`WindowKind::PopUp` + backdrop** — verify Mica/Blurred applies to popup windows on Windows; else apply `DWMWA_SYSTEMBACKDROP_TYPE` manually on the HWND.
- **Win+V takeover** — Windows updates occasionally change Explorer's hotkey handling; keep Alt+V as a documented fallback.
