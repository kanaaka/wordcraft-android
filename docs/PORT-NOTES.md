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

## 2026-10-13 — A vendored winit, so a mouse wheel is not swallowed

`touch_scroll.rs` covers fingers. A **mouse or trackpad** attached to the tablet was still dead: winit's
Android backend drops `DeviceEvent::Motion`-style actions outright, so the wheel and hover events never
reach the app, on any of these ports.

WordCraft carries the fix as a **vendored winit** — `vendor/winit/`, patched to forward those motion
actions, with `[patch.crates-io] winit = { path = "vendor/winit" }` in the workspace manifest. That is
why this repository ships a `vendor/` directory and why the build recipe copies it in: without it the
manifest does not resolve. The other ports read a pointer's wheel events the same way and would need
the same vendored crate; only this one has it so far.

**Checked:** the workspace resolves and the crate compiles for `aarch64-linux-android` with the
vendored winit in place (this is the tree the released APK was built from).

## 2026-10-13 — `adjustResize`, asked for through the theme

`android:windowSoftInputMode` has no home in cargo-apk's manifest schema, but it is a *themeable*
attribute (the platform's own base theme sets it as an item), so both theme variants now carry
`<item name="android:windowSoftInputMode">adjustResize</item>`. It asks the system to shrink the window
for the keyboard and the taskbar instead of drawing them over the app.

**What it does and does not achieve**, measured with `dumpsys window`: the window manager honours the
flag (`sim={adjust=resize}`), but nothing consumes the insets (`mImeInsetsConsumed=false`) — targeting
SDK 35 puts the app in charge of them, and consuming them needs a Java-side `WindowInsets` call this
project has no bridge for. It is kept because it is the right thing to ask for, and other Android
versions and devices do act on it.

## 2026-10-13 — Two-finger scrolling (and the wheel Android never sends)

**The complaint:** on the tablet a finger drag selected text instead of moving the page, and there was
no way to scroll at all.

**Why:** every app in this suite scrolls from `egui::Event::MouseWheel` — `WordCraft`'s `scroll.rs` is
explicit about the contract (pixel deltas move the page 1:1, `TouchPhase::End` on the last event is the
"fingers lifted" signal that starts the kinetic coast, wheel notches count as three lines). None of
that runs on Android, because winit's Android backend has no mouse and therefore emits no wheel
events at all (`src/platform_impl/android/mod.rs` sends six window events and a wheel is not one of
them). Touch arrives as `WindowEvent::Touch`, which egui-winit forwards as `egui::Event::Touch` *and*
turns into an emulated mouse pointer — so a finger drags a selection and the page never moves.

**The change — `src/touch_scroll.rs` (new, 231 lines, 3 tests), wired into the shell's
`raw_input_hook`:**

- one finger: untouched, so dragging still selects — the emulated pointer works as before;
- two or more fingers: the centroid's movement becomes `Event::MouseWheel` with
  `MouseWheelUnit::Point` and `TouchPhase::Move`;
- a final `TouchPhase::End` wheel event when the fingers lift, which is what makes the app coast to a
  stop instead of stopping dead;
- while a two-finger gesture is live the emulated **pointer button** events are dropped, or the first
  finger would drag a selection across the page at the same time as it scrolls. Pointer *position*
  events are kept, because the apps only scroll while the pointer is over the canvas;
- the touch events themselves stay in the batch: egui's pinch handling reads them, so a pinch still
  zooms and a two-finger *pan* moves nothing but the page.

**Ctrl+wheel zoom** works too, which is why `modifiers.rs`'s `apply()` now returns the state it tracked
(`egui::Modifiers`) instead of nothing: the bridge stamps it onto the wheel event it synthesises, and
the touch events carry it as well.

**Checked:** the three tests in `touch_scroll.rs` (one-finger pass-through, two-finger centroid →
wheel, the lift event) run on the host with `cargo test -p wordcraft-android`; the shell compiles for
`aarch64-linux-android`; and the string table of the `.so` inside the released APK carries the module.
On-device behaviour is the porter's tablet test, not an automated one.

## 2026-10-13 — `modifiers.rs` returns the tracked state

`apply(&mut self, raw)` now returns the `egui::Modifiers` it folded into the frame's events. The
synthesised two-finger wheel event has to carry modifiers itself (Ctrl+wheel is zoom in these apps,
plain wheel is scroll), and only the tracker knows them, so the state is handed back rather than
recomputed. Nothing else about the modifier handling changed.

## 2026-10-13 — The *Fonts* folder, in the shells that did not have one

PhotoCraft and VectorCraft already pointed their platform-font hook at a USB-visible
`Android/media/<package>/Fonts`; this shell did not, so there was no way to install a font on the
tablet at all.

The shell now creates `<App media>/Fonts`, sets `HOME` to the app's media directory, and symlinks
`.fonts` and `.local/share/fonts` to that one folder, so every path a font scan is likely to consult
lands in the same visible place. A `.ttf`/`.otf` dropped in over a cable appears in the app's font
list on the next scan. `Android/media` rather than `Android/data`, deliberately: Android 11+ hides
`Android/data` from MTP.

## 2026-10-13 — The picker no longer tells a *granted* user to grant access

The empty-folder message in shared storage used to end with "Grant *All files access*…", and it was
shown to everyone, including users who had already granted it. The trigger had been
`/storage/emulated/0/Android/data` failing to list — **measured on the emulator: that folder stays
unlistable to the app on API 35 even with the grant**, so the tell was wrong and the advice was
insulting to the one user who had done what it asked.

Now the note is conditional on the folder being shared storage, is worded as a possibility ("If your
files are here, the app may lack shared-storage access…"), and leaves the decision to the user,
because the grant cannot be probed from inside the app.

The USB tip became folder-dependent for the same reason — Android 11+ hides `Android/data` from MTP,
so a file copied there over a cable never arrives and the folder merely looks empty (which reads as a
picker bug), while `Android/media` *is* reachable over a cable. Saying the wrong one of those two is
worse than saying nothing.
## What is deliberately *not* in the patch

`Cargo.lock` is excluded on purpose: it moves only because the port crate is a new workspace member,
and `cargo` regenerates it on the first build. The patch contains only changes that matter to a reader
— every tracked file the port touches, plus any new file that has to live inside an upstream directory.

## Not verified here

The APK attached to **v0.1.1** was built from exactly this tree on 2026-10-13 (upstream pin + the patch + the port crate + the vendored winit), and the checks in the README's verification table were run against it. What that does *not* buy is a runtime claim: it was not launched on a device or emulator as part of publishing, so every behavioural statement below rests on the hand-testing the porter did on an arm64 tablet. The fixes above came
from the porter's hand-testing on an arm64 tablet — the bugs they fix were reported there — but there
is no automated device test here, and "verified" in the README means source-side checks plus that
testing, not a fresh run of this exact binary.
