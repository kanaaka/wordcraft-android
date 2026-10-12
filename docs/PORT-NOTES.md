# Port notes — WordCraft on Android

Upstream: [`storytold/wordcraft`](https://github.com/storytold/wordcraft) @
`2ffba93b5189410a415970be179dcf481092d756` (v0.5.0)
Published APK: `wordcraft-0.5.0-arm64-v8a-v4.apk`

The port is `apps/wordcraft-android/` (`src/lib.rs` 463 lines, `src/file_picker.rs` 403,
`src/modifiers.rs` 92, `src/prefs_watch.rs` 134) plus **one small upstream change** — an 8-line seam
in `crates/ui-egui/src/lib.rs` that lets a host open a second window. Everything below is the edit
log: what was wrong, why, what changed, how far it was checked.

## 1. The shell crate

`wordcraft_ui_egui::Services` is a plain struct of `Option<Box<dyn Fn…>>` fields deriving `Default`,
so a host supplies what it can and omits the rest. The desktop binary and the browser shell fill it
their own way; Android is the third. Details that are load-bearing in every port of this set:

- **`[lib] name = "wordcraft"` + `crate-type = ["cdylib"]`** → `lib/arm64-v8a/libwordcraft.so`, and
  cargo-apk points the injected `android.app.lib_name` at the artifact name (`-` → `_`). A mismatch
  installs and then dies on launch.
- **`options.android_app = Some(app)`** — `eframe::run_native` errors without it.
- **A file logger in the app's own data directory** — `logcat` needs adb, which a sideloaded build
  does not have.
- **Preferences under `internal_data_path()`**, including the recent-documents list, which lives in
  `ui.json`.

## 2. Two documents side by side — a second Activity (`new_window`)

WordCraft's *Window ▸ New Window* command is the way to review two documents at once, and on Android
the only thing that can show two windows of the same app is **two Activities**. The command could not
reach that: it opened a dialog, because the UI crate had no way to say "a host opened another window".

The upstream change is exactly that seam — a `Services::new_window: Option<Box<dyn Fn()>>` field, and
`"newWindow"` running it when a host supplies one (falling back to the dialog when nothing does):

```rust
"newWindow" => match &self.services.new_window {
    Some(open) => open(),
    None => self.dialog = dialogs::Dialog::open(what, self),
},
```

The shell implements `new_window()` with JNI: it keeps the `AndroidApp` handle it was handed at
startup (this is the one platform action that happens *after* startup, so the handle has to be kept)
and calls `startActivity` with the app's own `android.app.NativeActivity` component and
`FLAG_ACTIVITY_NEW_TASK`. Each window then runs its own event loop and its own document; the count of
live loops is what makes quitting one window safe (§4).

## 3. Quitting one window must not take the other with it

Before this, Exit was `std::process::exit(0)` — correct for the single-window case, wrong the moment
two windows exist: the process is shared, so the second window dies with the first.

`prefs_watch` counts the live event loops. When Exit is confirmed and more than one window is open,
the shell asks *its own* loop to end instead (`ViewportCommand::Close` plus clearing the app's quit
flag, so `run_native` returns and `android_main` returns) and lets android-activity retire that
Activity; only the last window ends the process. The count cannot lie, because it is decremented when
a loop actually returns.

## 4. Preferences are written while the app runs (`prefs_watch`)

Nothing on Android arrives at the place the desktop writes preferences. The desktop saves them on its
way out; Android gives no such moment, so a swipe from Recents took every preference with it —
including the **recent-documents list**, which is why saved documents kept vanishing from the Start
screen.

`prefs_watch` writes the preferences from the frame loop: change-detected (a hash), at most every two
seconds, and atomically (a temp file and a rename, so a kill mid-write cannot leave a truncated JSON
file where the preferences used to be). The app's own save-on-exit is still there for the clean path.

## 5. The Activity is not recreated (`config_changes`)

`[package.metadata.android.application.activity] config_changes = "orientation|keyboardHidden|…"`
so a rotation, a keyboard appearing, a density or font-scale change, or the window being resized does
not recreate the Activity. A recreated Activity destroys the native window while the render thread
lives on — a blank surface — and that is the symptom the porter reported when WordCraft was put into
split screen. The declaration is this port's defence against it.

## 6. Ctrl, Shift and Alt did nothing — `src/modifiers.rs`

From winit's own source: its Android backend emits only six window events and never `ModifiersChanged`.
`egui-winit` updates modifiers from that event and nowhere else, and egui carries modifiers as an
*event*, so `ctx.input(|i| i.modifiers)` returns the all-false default for the whole run — Ctrl+S,
Ctrl+Z and Shift-click are dead.

The key events do arrive, so the shell tracks the modifiers in `raw_input_hook` and re-injects them
the way egui expects: folded into the held state, stamped onto every `Key`/`PointerButton` event, plus
one `ModifiersChanged` per frame. State is carried **between** frames, because the press and the click
usually land in different ones.

## 7. File ▸ Open and Save As did nothing — `src/file_picker.rs`

`rfd` has no Android backend (its `backend.rs` compiles no module for a target that is neither
Linux/BSD, macOS, Windows nor wasm32; the build fails with `the trait bound FileDialog:
FilePickerDialogImpl is not satisfied`), and the Storage Access Framework needs
`startActivityForResult`, which `android-activity` 0.6 does not expose. So the picker is drawn with
egui, between frames, on WordCraft's `Services::file_dialog` seam — the **reply-callback** shape: the
shell stashes the request and its reply, returns at once, and answers from a later frame.

The answer carries a **path**, not finished bytes, and that is deliberate: with bytes only, the
document never learns which file it is — the title bar stays "Untitled", Ctrl+S asks for a location
again, and closing prompts to save what was just saved. A save dialog returns `dir.join(name)` because
the app's atomic write needs the parent directory to exist.

The picker itself follows the rules the sibling ports also follow: sized from `ctx.content_rect()`,
enlarged padding for fingers, a repaint requested while open, listings and readability probes cached
per visit rather than per frame, locked roots left tappable so they can name the permission they need,
the filter shown as the union of every extension the app asked for with **"Show all files"** and a
count of the hidden ones, and the app's own directory as the default so the first save always works
with no permission at all.

## 8. Permissions, icon, theme

`MANAGE_EXTERNAL_STORAGE` (granted by hand under Settings ▸ Special access; Google Play restricts it
to file-manager-class apps, which does not apply to a sideload), `READ_MEDIA_IMAGES` for Android 13+,
and the two legacy storage permissions capped with `max_sdk_version`. `res/values/styles.xml` carries
the same two-variant edge-to-edge theme as its siblings, needed because `targetSdkVersion 35` puts the
app under a transparent status bar on Android 15, and `res/mipmap-*` carries the launcher icon.

## Upstream file this port changes

| File | Change | Why |
|---|---|---|
| `crates/ui-egui/src/lib.rs` | `Services::new_window` (+ the `"newWindow"` handler falling back to the dialog) | lets a host open a second window; on Android that is a second Activity (§2) |

Nothing else upstream is touched.

## Not verified here

The APK in Releases was not re-run on a device while publishing this repository. The fixes above came
from the porter's hand-testing on an arm64 tablet — the bugs they fix were reported there — but there
is no automated device test here, and "verified" in the README means source-side checks plus that
testing, not a fresh run of this exact binary.
