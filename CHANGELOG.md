# Changelog

## v0.1.1 — 2026-10-13 (scrolling, modifiers, fonts, picker wording)

| | |
|---|---|
| APK | `wordcraft-0.5.0-arm64-v8a-20261013.apk` (19.9 MB) — supersedes the v0.1.0 asset, same signing key |
| sha256 | `9c21e2ab11f91c89bc00188d685d55b800688eabf2d531a335ea08a9caf619f8` |
| Built from | upstream `storytold/wordcraft` @ `2ffba93b5189410a415970be179dcf481092d756` + this repository at the commit that released it |
| Package | `ai.storyteller.wordcraft` — versionName `0.5.0`, versionCode `16778496`, minSdk 26, targetSdk 35 |
| ABI | `arm64-v8a` only |

### Changed
- **Two-finger scrolling.** A two-finger drag now scrolls the document (synthesised `MouseWheel`
  events from the touch centroid, with the lift event that starts the kinetic coast); one finger still
  selects, a pinch still zooms, and Ctrl+wheel zooms.
- **A mouse or trackpad wheel works.** winit's Android backend drops motion actions, so the wheel and
  hover events never reached the app; this repository now carries a **vendored, patched winit**
  (`vendor/winit/`) that forwards them, referenced by a `[patch.crates-io]` entry in the workspace
  manifest. The build recipe copies it in.
- **A *Fonts* folder** at `Android/media/<package>/Fonts`, with `.fonts` and `.local/share/fonts`
  symlinked to it, so a `.ttf` copied in over a USB cable is picked up — previously there was no way
  to install a font on the tablet at all.
- **Picker wording corrected** (see the notes): no grant advice to a user who already granted, and a
  USB tip that matches the folder.
- **`android:windowSoftInputMode=adjustResize`** asked for through both theme variants, so the system
  shrinks the window for the keyboard and the taskbar. On Android 15 the flag is honoured but nothing
  consumes the insets yet — see the notes for exactly what it does and does not achieve.

### Notes
The build was made from exactly the source in this repository (upstream pin + the patch + the port
crate) and signed with the same local test key as v0.1.0, so it installs over it.


## v0.1.0 — first published port (WordCraft 0.5.0)

| | |
|---|---|
| APK | `wordcraft-0.5.0-arm64-v8a-v4.apk` (19.9 MB) |
| sha256 | `726b570c652a9630cdc0cdc499e9157890ced6838ffdcae68937f2417e1fccfa` |
| Built from | upstream `storytold/wordcraft` @ `2ffba93b5189410a415970be179dcf481092d756` + `patches/0001-wordcraft-android-port.patch` + `apps/wordcraft-android/` |
| Package | `ai.storyteller.wordcraft` — versionName `0.5.0`, minSdk 26, targetSdk 35 |
| ABI | `arm64-v8a` only |
| Signature | throwaway local test key, not an official key |

### In this release
- Android shell crate: NativeActivity entry point, `Services` implementation, file logger, preferences
  under the app's own data directory.
- **Window ▸ New Window opens a real second window** (a second Activity over JNI), so two documents can
  be reviewed side by side — with a quit path that ends one window instead of the process.
- **Preferences written while the app runs** (change-detected, atomic), so the recent-documents list
  and every other preference survive an Android kill rather than only a clean exit.
- **Ctrl / Shift / Alt work** — the platform reports no modifiers, so they are tracked from key events
  and re-injected.
- **Activity not recreated** on rotation, docking, split-screen resize, density or font-scale change
  (`config_changes`).
- In-app file picker for Open and Save As, returning the chosen **path**, with cached listings, "Show
  all files", hidden-file count and self-explaining locked roots.
- Launcher icon, storage permissions, edge-to-edge theme variant.


### Known issues
See *Known gaps* in the [README](README.md) and [`docs/PORT-NOTES.md`](docs/PORT-NOTES.md).
