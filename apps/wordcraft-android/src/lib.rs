//! WordCraft on Android (arm64-v8a): the real engine and egui shell, running on winit's
//! NativeActivity backend.
//!
//! # What this crate is
//!
//! A shell around the *unmodified* WordCraft crates — `wordcraft-engine` (the document model,
//! the command registry, file formats) and `wordcraft-ui-egui` (the ribbon, rulers, panes,
//! Backstage and every dialog). Nothing in those crates is patched. This crate supplies only
//! what `apps/wordcraft` would normally supply: the [`Services`] implementation, the
//! preferences file, the window options and the entry point. The same shape the PhotoCraft and
//! VectorCraft ports used.
//!
//! # Why a separate crate, and not a feature on `apps/wordcraft`
//!
//! That crate cannot build for Android as written: it carries native dialogs (`rfd`), the OS
//! clipboard (`arboard`), a control server and a system print flow, none of which exist there.
//! Verified with `cargo check -p wordcraft --target aarch64-linux-android`: `arboard` stops
//! first, with `cannot find Set in module platform` — it has no Android backend at all, and
//! `rfd` fails the same way with `the trait bound FileDialog: FilePickerDialogImpl is not
//! satisfied`.
//!
//! `wordcraft_ui_egui::Services` is a plain struct of `Option<Box<dyn Fn…>>` fields that derives
//! `Default`, so the fix is not to patch the app but to build a platform that supplies what it
//! can and omits the rest — which is exactly what `Services` was shaped for. Its `ui-egui` is
//! Android-clean: `cargo check -p wordcraft-ui-egui --target aarch64-linux-android` passes with
//! no changes.
//!
//! # Feature notes
//!
//! - `android-native-activity` **must** be on. Without a backend feature, `android-activity` is
//!   pulled with no backend and the build fails with `file not found for module activity_impl`
//!   plus `Either "game-activity" or "native-activity" must be enabled as features`.
//! - `accesskit` **must** be off, and here it is unreachable: `ui-egui` has no eframe dependency
//!   at all, so this crate is the only eframe user in the graph and never asks for it.
//! - `NativeOptions::android_app` must be `Some(..)`: eframe hands it to
//!   `EventLoopBuilderExtAndroid::with_android_app` and returns an error without it.
//!
//! # What Android cannot do yet
//!
//! Honest gaps, not oversights:
//!
//! - **File dialogs are drawn by the app itself** ([`file_picker`]). WordCraft's `Services`
//!   already has the asynchronous seam for exactly this (`file_dialog`, a request whose answer
//!   arrives later through a channel — the desktop shell uses it so the window never freezes),
//!   so Open, Save As, Export, Insert Picture and the mail-merge recipient list all work: the
//!   picker browses the app's own folder with no permission at all, and offers a shared-storage
//!   root only when it can actually read it. Android's *own* picker is still future work: the
//!   Storage Access Framework needs `startActivityForResult`, which `android-activity` 0.6 does
//!   not expose.
//! - **No system clipboard.** `arboard` has no Android backend, so `clipboard_picture` stays
//!   `None`: Paste still handles text through egui, but a picture copied in another app is not
//!   offered.
//! - **No system print.** `print` stays `None`, so File ▸ Print has nowhere to hand the PDF.
//! - **No stylus pressure or tilt.** A pen degrades to ordinary touch; Android does expose both
//!   on `MotionEvent`, so a backend is writable, but it does not exist yet.
//! - **No system title bar.** `integrated_titlebar` is left off, so the app keeps its own title
//!   bar with the Quick Access Toolbar and ribbon — which is what the tablet shows, fullscreen.
//!
//! Editing itself is unaffected: the document model, layout, the CPU rasteriser and the file
//! formats are platform-neutral, and the fonts are `include_bytes!`d, so text works with no
//! system font folder at all.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use wordcraft_engine::Session;
use wordcraft_ui_egui::{FileDialogRequest, Services, UiState, WordApp};

mod file_picker;
mod modifiers;
mod prefs_watch;

/// The app's private data directory, captured in `android_main` before any service closure
/// runs: `<internal storage>/Android/data/ai.storyteller.wordcraft/files`.
///
/// `internal_data_path()` needs no runtime permission — it is the app's own sandbox — which is
/// why the preferences and the log land here rather than in shared storage.
static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

/// WordCraft's preferences: a `ui.json` in the app's own folder, the same file the desktop
/// keeps in `%APPDATA%\WordCraft` (or its XDG/macOS equivalent) through
/// `wordcraft_control_key::settings_dir()`.
fn prefs_file() -> Option<PathBuf> {
    DATA_DIR.get().map(|d| d.join("ui.json"))
}

/// Mirrors the desktop `load_prefs`.
///
/// The restored window geometry is dropped: there is no OS window to move on Android, and the
/// desktop shell is what tracks it (`WindowGeometry::track`) — keeping a stale rectangle would
/// only feed the "cascade a new window" arithmetic.
fn load_prefs(app: &mut WordApp) {
    if let Some(p) = prefs_file()
        && let Ok(bytes) = std::fs::read(&p)
        && let Ok(ui) = serde_json::from_slice::<UiState>(&bytes)
    {
        app.apply_prefs(ui);
    }
    app.ui.window = None;
}

/// How often the preferences are looked at; a change is written at the next look. Short enough
/// that nothing a user would miss is stale, long enough to cost nothing while the app is idle.
const PREFS_EVERY: Duration = Duration::from_secs(2);

/// What goes to disk: the same `ui.json` the desktop writes on its way out, which is also where
/// the recent-documents list lives.
fn prefs_bytes(app: &WordApp) -> Vec<u8> {
    serde_json::to_vec_pretty(&app.prefs()).unwrap_or_default()
}

/// The file types Open offers, as on the desktop: pictures for Insert/Change Picture, recipient
/// lists for Select Recipients ▸ Use an Existing List…, documents otherwise.
///
/// The desktop's trailing "All files" filter (`&["*"]`) is dropped: it is a wildcard the picker
/// has nothing to match against, and listing every file on a tablet's shared storage to sort
/// through is worse than the filter.
fn open_extensions(purpose: &str) -> Option<Vec<String>> {
    match purpose {
        "picture" => file_picker::extensions(&[&["png", "jpg", "jpeg", "gif", "webp", "bmp"]]),
        "recipients" => file_picker::extensions(&[&["csv", "tsv", "txt", "xlsx", "xlsm", "ods"]]),
        _ => file_picker::extensions(&[&["docx", "docm", "dotx", "dotm", "doc", "dot", "odt", "rtf", "txt", "md", "html", "htm", "tex", "json"]]),
    }
}

/// The `Services::file_dialog` hook: what the desktop shell fills with `rfd::AsyncFileDialog`.
///
/// WordCraft asks for a dialog by sending a [`FileDialogRequest`] and gets a `Receiver` back; it
/// polls that once a frame in [`WordApp::logic`] and finishes what the dialog was for when the
/// path arrives (open the document, write it and carry on with the pending New/Open/Close,
/// export, insert the picture, load the recipients). `None` means cancelled.
///
/// That is the same asynchronous contract the picker keeps, so the two are bridged with one
/// waiting thread: the picker answers `Vec<PathBuf>`, and the thread turns it into the
/// `Option<String>` the app is polling for. One thread per pick, and it ends with the request.
fn file_dialog(req: FileDialogRequest) -> Receiver<Option<String>> {
    let (kind, title, extensions, suggested) = match req {
        FileDialogRequest::Open { purpose } => (file_picker::Kind::Open, "Open \u{2014} Android", open_extensions(&purpose), None),
        FileDialogRequest::Save { name } => (file_picker::Kind::Save, "Save As \u{2014} Android", None, Some(name)),
    };
    let paths = file_picker::request(kind, title, extensions, suggested);
    let (answer, rx) = std::sync::mpsc::channel();
    let waiting = std::thread::Builder::new().name("file dialog".into()).spawn(move || {
        let picked = paths.recv().ok().and_then(|v| v.into_iter().next()).map(|p| p.to_string_lossy().into_owned());
        let _ = answer.send(picked);
    });
    if let Err(e) = waiting {
        // The dropped sender reads as "cancelled" in the app, which is the honest outcome.
        log::error!("couldn't start the file-dialog thread: {e}");
    }
    rx
}

/// The platform-neutral half of the desktop `services()`.
///
/// `file_dialog` carries the whole Open/Save As/Export/Insert-Picture path; everything else the
/// desktop fills — `pick_open`/`pick_save` (the blocking dialogs), the web `open_async`/`inbox`/
/// `download`, `print` and the clipboard — is deliberately left out. Leaving `pick_open` and
/// `pick_save` out is what makes the app use `file_dialog` instead of a blocking dialog, and
/// leaving `download` out is what makes File ▸ Save fall through to Save As rather than looking
/// for a browser to hand bytes to.
// This window's `AndroidApp`, kept for what the app asks of the platform *after* start-up —
// opening a second window, today.
//
// android-activity warns against holding an `AndroidApp` in global state because it dies with its
// Activity; this is per-thread state that `android_main` fills in and clears again as its loop
// runs and ends, so it never outlives the Activity it belongs to.
#[cfg(target_os = "android")]
thread_local! {
    static APP: std::cell::RefCell<Option<winit::platform::android::activity::AndroidApp>> =
        const { std::cell::RefCell::new(None) };
}

/// Open a second window on this app: a second Activity, so two documents can be put side by side on
/// the tablet (Android's split screen gives one window per task).
///
/// No manifest metadata does this — `documentLaunchMode` has no home in `cargo-apk`'s schema — and
/// none is needed if the intent says it instead: `FLAG_ACTIVITY_NEW_DOCUMENT` with
/// `FLAG_ACTIVITY_MULTIPLE_TASK` *is* that attribute ("a document of its own, in its own task, even
/// though an instance is running") and the launch intent has to come from us. The component is
/// asked of the running Activity (`getComponentName()`) rather than spelled out from a package name
/// and the launcher class, so nothing here has to stay in agreement with the Cargo metadata by
/// hand; the flags are read from the `Intent` class rather than copied as numbers.
///
/// A launch that fails is logged and nothing else happens: the menu entry that called this has
/// already returned, so there is no one to tell.
#[cfg(target_os = "android")]
fn new_window() {
    let Some(app) = APP.with(|slot| slot.borrow().clone()) else {
        log::error!("no AndroidApp in this window: cannot open another");
        return;
    };
    // SAFETY: `vm_as_ptr` is the JavaVM this process is already attached to, and `JavaVM::from_raw`
    // is a singleton behind a non-null assert — passing the pointer is all it takes.
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) };
    let opened = vm.attach_current_thread(|env| -> jni::errors::Result<()> {
        use jni::objects::{JObject, JValue};
        use jni::signature::{RuntimeFieldSignature, RuntimeMethodSignature};
        use jni::strings::JNIStr;

        // Names and signatures, each written once. A C string literal cannot carry an interior NUL
        // or invalid MUTF-8, so `from_cstr` cannot fail on one.
        let name = |n: &'static std::ffi::CStr| JNIStr::from_cstr(n).expect("a literal name");
        let method = RuntimeMethodSignature::from_str;
        let get_component = method("()Landroid/content/ComponentName;")?;
        let set_component = method("(Landroid/content/ComponentName;)Landroid/content/Intent;")?;
        let add_flags = method("(I)Landroid/content/Intent;")?;
        let start_activity = method("(Landroid/content/Intent;)V")?;
        let no_args = method("()V")?;
        let int_field = RuntimeFieldSignature::from_str("I")?;

        // SAFETY: this is this Activity's own reference, valid for as long as its `AndroidApp` is.
        // `JObject` does not own what it points at — it has no `Drop`, which is exactly why
        // android-activity's own JNI example wraps this pointer this way and warns only against
        // `Global`/`Auto`, which would delete a reference the platform still holds.
        let activity = unsafe { JObject::from_raw(env, app.activity_as_ptr() as jni::sys::jobject) };

        let component =
            env.call_method(&activity, name(c"getComponentName"), get_component.method_signature(), &[])?.l()?;
        let intent_class = name(c"android/content/Intent");
        let intent = env.new_object(intent_class, no_args.method_signature(), &[])?;
        env.call_method(
            &intent,
            name(c"setComponent"),
            set_component.method_signature(),
            &[JValue::Object(&component)],
        )?;
        // The pair of flags, named rather than numbered: a new document in a new task, and do it
        // even though this app already has a window.
        let flags = env
            .get_static_field(intent_class, name(c"FLAG_ACTIVITY_NEW_DOCUMENT"), int_field.field_signature())?
            .i()?
            | env
                .get_static_field(intent_class, name(c"FLAG_ACTIVITY_MULTIPLE_TASK"), int_field.field_signature())?
                .i()?;
        env.call_method(&intent, name(c"addFlags"), add_flags.method_signature(), &[JValue::Int(flags)])?;
        env.call_method(
            &activity,
            name(c"startActivity"),
            start_activity.method_signature(),
            &[JValue::Object(&intent)],
        )?;
        Ok(())
    });
    match opened {
        Ok(()) => log::info!("asked the system for a second window"),
        Err(e) => log::error!("couldn't ask the system for a second window: {e}"),
    }
}

/// What this host supplies: the in-app file picker, and — on Android, the only place with a second
/// window to open — [`new_window`].
#[cfg(target_os = "android")]
fn android_services() -> Services {
    Services {
        file_dialog: Some(Box::new(file_dialog)),
        new_window: Some(Box::new(new_window)),
        ..Default::default()
    }
}

/// The host build has no Activity to open a window on; it exists so `cargo check` (and `cargo
/// clippy`, and the lint pass in CI) can see this crate's platform-independent parts at all.
#[cfg(not(target_os = "android"))]
fn android_services() -> Services {
    Services { file_dialog: Some(Box::new(file_dialog)), ..Default::default() }
}

/// A minimal file logger, so a sideloaded APK leaves a readable trace.
///
/// `logcat` is only reachable over adb; a file in the app's own data directory can be pulled
/// with any file manager, which is what a sideloaded build needs.
struct FileLogger {
    sink: Mutex<Option<std::fs::File>>,
}

impl log::Log for FileLogger {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }
    fn log(&self, record: &log::Record) {
        if let Ok(mut guard) = self.sink.lock() {
            if let Some(f) = guard.as_mut() {
                use std::io::Write;
                let _ = writeln!(f, "[{}] {} {}", record.level(), record.target(), record.args());
            }
        }
    }
    fn flush(&self) {}
}

static LOGGER: OnceLock<FileLogger> = OnceLock::new();

fn init_logging(dir: &Path) {
    let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("wordcraft.log")) else {
        return;
    };
    let logger = LOGGER.get_or_init(|| FileLogger { sink: Mutex::new(None) });
    if let Ok(mut guard) = logger.sink.lock() {
        *guard = Some(file);
    }
    if log::set_logger(logger).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
}

/// The Android entry point.
///
/// `android-activity` spawns a dedicated thread for this and calls it with the activity handle.
/// `#[unsafe(no_mangle)]` with the Rust ABI is the required shape (edition 2024 requires the
/// `unsafe(..)` wrapper around `no_mangle`), and it is the symbol the injected NativeActivity
/// looks for.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub fn android_main(app: winit::platform::android::activity::AndroidApp) {
    // Count this window while its loop runs, so a quit can tell whether ending it should end the
    // process or only this window (see `prefs_watch`).
    let _window = prefs_watch::counted_window();
    // Kept for the one platform action that comes later (see `new_window`); cleared when this
    // window's loop ends.
    APP.with(|slot| *slot.borrow_mut() = Some(app.clone()));
    let data_dir = app.internal_data_path().unwrap_or_else(|| PathBuf::from("."));
    let _ = std::fs::create_dir_all(&data_dir);
    let _ = DATA_DIR.set(data_dir.clone());
    init_logging(&data_dir);
    log::info!("WordCraft {} starting on Android; data dir {}", env!("CARGO_PKG_VERSION"), data_dir.display());
    // The app's external files directory: always readable and writable with no permission, so
    // the picker offers it as a root and Save falls back to it (see `file_picker`).
    if let Some(ext) = app.external_data_path() {
        let _ = std::fs::create_dir_all(&ext);
        let _ = file_picker::EXT_DIR.set(ext);
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("WordCraft").with_app_id("ai.storyteller.wordcraft"),
        // Required on Android: eframe hands this to
        // `EventLoopBuilderExtAndroid::with_android_app` and errors without it.
        android_app: Some(app),
        ..Default::default()
    };

    let result = eframe::run_native(
        "WordCraft",
        options,
        Box::new(|cc| {
            let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), android_services());
            load_prefs(&mut app);
            // No caption buttons: the app draws its own title bar (Quick Access Toolbar, ribbon).
            // `integrated_titlebar` is the macOS switch that leaves room for the traffic lights
            // inside that bar; on Android nothing draws them, so it stays off.
            app.integrated_titlebar = false;
            if let Some(rs) = &cc.wgpu_render_state {
                let info = rs.adapter.get_info();
                log::info!("wgpu adapter: {info:?}");
            } else {
                log::warn!("no wgpu render state");
            }
            Ok(Box::new(Shell {
                inner: app,
                modifiers: modifiers::Tracked::default(),
                prefs: prefs_watch::Writer::new(prefs_file(), PREFS_EVERY),
            }))
        }),
    );

    match result {
        // eframe::Result is Result<(), eframe::Error>; a normal exit happens when the process
        // ends or the window closes, which is why the app writes the line itself in that case.
        Ok(()) => log::info!("WordCraft exited normally"),
        Err(e) => log::error!("WordCraft stopped: {e}"),
    }
    APP.with(|slot| *slot.borrow_mut() = None);
}

/// Wraps the real app so the platform can act on the frame around it — here, ending the process
/// when File ▸ Exit was chosen.
///
/// The same shape as the desktop shell (`apps/wordcraft/src/main.rs`) and the web build:
/// `WordApp` stays untouched and the platform-specific layer hangs off the outside. Every method
/// is forwarded, so the app sees no difference.
struct Shell {
    inner: WordApp,
    /// Modifier keys, tracked here because the platform never reports them: winit's Android
    /// backend sends no `ModifiersChanged` at all, which is why Shift+click and every Ctrl
    /// shortcut are dead without this (see `modifiers`).
    modifiers: modifiers::Tracked,
    /// The preferences, written as the app runs rather than only on the way out: Android never
    /// gives that ending, so a swipe from Recents would otherwise take every preference with it
    /// (see [`prefs_watch`]).
    prefs: prefs_watch::Writer,
}

impl Shell {
    /// File ▸ Exit, and any other path that sets `quit_requested`.
    ///
    /// The app does not close its own window: it sets the flag and expects the *host* to do it.
    /// The desktop shell answers with `ViewportCommand::Close`, and on macOS it ends the process
    /// itself. That answer is not available here: an Android app is closed by finishing its
    /// activity, which needs Java, and winit's Android backend stubs out the window API a close
    /// would go through — `set_visible`, `focus_window` and `pre_drag_notify` are no-ops,
    /// `drag_window` returns an error, `set_fullscreen` only warns — and winit 0.30 has no
    /// `request_close` at all, on any backend. So the process ends, exactly as the desktop shell
    /// ends it on macOS.
    ///
    /// Nothing can be unsaved by this point: `quit_requested` is only set once the "save
    /// changes?" prompt has been answered, so what is left is what `on_exit` would have done —
    /// save the preferences — and it is done here because a process exit runs no `on_exit`.
    fn quit_if_asked(&mut self, ctx: &egui::Context) {
        if !self.inner.quit_requested {
            return;
        }
        let bytes = prefs_bytes(&self.inner);
        self.prefs.store_now(&bytes);
        log::info!("quitting");
        log::logger().flush();
        if prefs_watch::windows() > 1 {
            // Another window of this app is open in this process (a second Activity). Killing the
            // process would take that window with it, so this one asks its own event loop to end:
            // `run_native` returns, `android_main` returns, and android-activity retires this
            // Activity — which is what it asks of an app after `MainEvent::Destroy`. `windows()`
            // cannot lie here: the count is only decremented when a loop actually returns.
            self.inner.quit_requested = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else {
            std::process::exit(0);
        }
    }
}

impl eframe::App for Shell {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.inner.logic(ctx);
        if self.prefs.due() {
            let bytes = prefs_bytes(&self.inner);
            self.prefs.store(&bytes);
        }
        self.quit_if_asked(ctx);
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        // Before the app, so the frame it sees already has the modifiers a desktop backend would
        // have reported (see `modifiers`).
        self.modifiers.apply(raw);
        self.inner.raw_input_hook(raw);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.inner.ui(ui);
        // After the app, so the picker floats above the ribbon and the page.
        file_picker::draw(ui.ctx());
        // File ▸ Exit is a menu click, so it is set during this call: check here, in the same
        // frame, rather than leaving it to the next `logic` — no repaint is guaranteed after a
        // click, and waiting for one is how Exit ends up doing nothing.
        self.quit_if_asked(ui.ctx());
    }

    fn on_exit(&mut self) {
        let bytes = prefs_bytes(&self.inner);
        self.prefs.store_now(&bytes);
    }
}
