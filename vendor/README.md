# Vendored crates

Permissively licensed crates we carry small patches for. Each directory keeps the upstream licence
files unchanged. Every patch is marked `WordCraft patch:` in the source, is covered by a WordCraft
test, and should be proposed upstream. Remove the vendored copy once upstream releases the fix.
Vendoring copyleft code is never allowed.

| Crate | Version | Licence | Patches | Test |
|---|---|---|---|---|
| winit | 0.30.13 | Apache-2.0 | (1) on Android, `MotionAction::Scroll` becomes `WindowEvent::MouseWheel`, and (2) `MotionAction::HoverMove` becomes `WindowEvent::CursorMoved`. Upstream's Android backend maps only `Down`/`Up`/`Move`/`Cancel` and drops everything else (`_ => None // TODO mouse events`), so a mouse wheel and a touchpad's two-finger scroll never reach the app — and on this platform a wheel is the *only* way an app scrolls, because winit emits no wheel event from any other input. The hover half is not optional: the apps in this suite scroll only while the pointer is over the canvas, and without hover events the pointer never follows a mouse. Nothing in a shell can work around either one, because the events do not exist. The published crate's asset files (`docs/res/*.svg`, `examples/data/*.png`) are left out, so its `window` example does not build | `apps/wordcraft-android` `tests::winit_carries_the_android_scroll_and_hover_patch` keeps the patch in the build; the wheel and the trackpad themselves need the tablet |
