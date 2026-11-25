# mocli

A clipboard history for Windows, in the style of Raycast's. Press `Win+V`: the
popup opens next to the text you are typing, **without taking focus**, and
`Enter` pastes. Built in Rust with [gpui-kit](https://gpui-kit.com).

## Install

Download from [Releases](https://github.com/bennett-sh/mocli/releases):

- `mocli-<version>-x64-setup.exe` installs for the current user, no
  administrator rights needed.
- `mocli-<version>-x64-portable.zip` runs from any folder and keeps its
  settings and history in a `data` folder next to it.

Windows 10 2004 or later, 64-bit. The builds are not code signed, so SmartScreen
asks before the first start.

## Use

| Key | Action |
|---|---|
| type | Search (including text found in images) |
| `↑` `↓` `PgUp` `PgDn` | Move through the list |
| `Enter` / `Ctrl+Enter` | Paste / copy only |
| `Shift+Enter` | Paste as plain text |
| `Ctrl+1` … `Ctrl+9` | Paste that entry |
| `Ctrl+K` | All actions for the entry |
| `Ctrl+P` | Pin or unpin |
| `Ctrl+Backspace` / `Del` | Delete (with an empty search) |
| `Ctrl+E` | Edit, then paste |
| `Ctrl+O` | Open the link, file or image |
| `Tab` / `Shift+Tab` | Change the type filter |
| `Ctrl+,` | Settings |
| `Esc` | Clear the search, then close |

It keeps text, code, links, email addresses, colors, images, SVG and files, each
with a preview and the program it came from.

## Settings

Open them from the tray icon or with `Ctrl+,`, or edit
`%APPDATA%\mocli\config.toml`; changes apply as soon as the file is saved.

## Privacy

- The history is stored **unencrypted** in `%LOCALAPPDATA%\mocli`.
- Content that apps mark as private is never saved, nor are copies from
  1Password, Bitwarden and KeePass, nor anything that looks like a card number,
  private key or access token. These checks cannot catch every secret; pause
  capture from the tray when in doubt.
- Nothing leaves your computer. The one exception is link previews, which are
  off unless you switch them on.

## Limitations

- Programs running as administrator neither receive pasted text nor trigger the
  shortcut.
- Input methods (for example Chinese or Japanese) need the "focus moves to the
  popup" setting.

## Develop

```powershell
cargo run
cargo test
pwsh scripts/package.ps1 -SkipInstaller   # portable zip in dist/
```

End-to-end checks drive the real app with real input, so they need an unlocked,
idle desktop:

```powershell
cargo build; pwsh -STA -File scripts/e2e-popup.ps1
```

The others are `scripts/e2e-capture.ps1` and `scripts/e2e-placement.ps1`. The
design and its history are in [PLAN.md](PLAN.md).

## Releases

Work happens on `dev`; pushing it lints, tests and builds the portable zip and
the installer. Pushing `main` publishes the version in `Cargo.toml` as the tag
`v<version>` with a changelog from the commit messages, reusing the files built
on `dev` for the same source.

Commit messages follow `type(scope): description` (`feat`, `fix`, `perf`,
`refactor`, `docs`, `test`, `build`, `ci`, `chore`; `!` marks a breaking
change):

- A **dot after the type**, such as `fix.(core): …`, lists the change in the
  changelog without publishing for it. A push with only dotted commits publishes
  nothing.
- A new version creates a new release.
- With an unchanged version, the release's tag moves to the new commit and its
  files and notes are replaced.
