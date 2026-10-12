# Changelog

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
