//! Writing the preferences while the app runs, not only on the way out.
//!
//! # Why this exists
//!
//! The desktop shells write their `ui.json` (and `prefs.json`) from `eframe::App::on_exit`, which
//! eframe calls when the event loop ends. **Android never gives that ending.** An app is swiped
//! away from Recents and the process is killed where it stands; a low-memory kill looks the same;
//! and winit's Android backend does not even forward the activity's `onDestroy` (its
//! `src/platform_impl/android/mod.rs` logs `TODO: forward onDestroy notification to application`
//! and carries on). So everything the app keeps between runs died with it: the recent-documents
//! list, the theme, the interface size, the language, the ribbon state — which a user sees as
//! "it forgot the file I just saved".
//!
//! So the preferences are written as the app runs: when the serialised state *differs* from what
//! was last written, and at most every [`Self::every`]. Comparing first means an idle app writes
//! nothing at all, and a change is on disk within the interval.
//!
//! # Why the write is atomic
//!
//! A second window of the same app (see the shell's new-window path — a second `android_main` in
//! the same process) writes *the same file*. A plain `fs::write` from two threads can interleave
//! into a half-written `ui.json`, and a half-written preferences file is every preference lost at
//! once. So each write goes to its own temporary name and is renamed into place, which is atomic
//! on the same filesystem: a reader sees either the old file or the new one.
//!
//! The same file means the same *content* rules as the desktop, which also has many windows
//! sharing one `ui.json`: the last writer wins. Nothing is corrupted, but two windows do not merge
//! their recent lists.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// The preferences file, how often to look at it, and what was written last.
pub struct Writer {
    path: Option<PathBuf>,
    every: Duration,
    due_at: Instant,
    last: Vec<u8>,
}

impl Writer {
    /// `every` is how often the state is looked at; a quick look costs a comparison, and a change
    /// is written as soon as one is due.
    pub fn new(path: Option<PathBuf>, every: Duration) -> Self {
        Writer { path, every, due_at: Instant::now() + every, last: Vec::new() }
    }

    /// Whether it is time to look, and there is somewhere to write.
    pub fn due(&mut self) -> bool {
        if self.path.is_none() {
            return false;
        }
        let now = Instant::now();
        if now < self.due_at {
            return false;
        }
        self.due_at = now + self.every;
        true
    }

    /// Write `bytes` if they are not what was last written. Called with what [`Self::due`] asked
    /// for, or with anything else at any time.
    pub fn store(&mut self, bytes: &[u8]) {
        if bytes.is_empty() || bytes == self.last {
            return;
        }
        let Some(path) = self.path.as_ref() else {
            return;
        };
        match write_atomic(path, bytes) {
            Ok(()) => self.last = bytes.to_vec(),
            Err(e) => log::warn!("couldn't write the preferences: {e}"),
        }
    }

    /// Write now, whatever was written last — for the paths that are not coming back: a quit, and
    /// `on_exit` if the host ever calls it.
    pub fn store_now(&mut self, bytes: &[u8]) {
        self.last.clear();
        self.store(bytes);
    }
}

/// How many windows of this app are running in this process.
///
/// Not every shell has a quit path to ask (`GridCraft` ends its process directly), so this is
/// allowed to go uncalled there rather than being cfg'd per app.
///
/// A second window is a second `android_main` on its own thread (a second Activity), so this
/// counts loops rather than apps: the shell registers one at the start and drops it when its loop
/// ends. It is what tells a window whether ending it should end the *process* (the last window) or
/// just itself.
static WINDOWS: AtomicUsize = AtomicUsize::new(0);

/// The number of windows this process has running, including the calling one.
///
/// Not every shell has a quit path to ask (`GridCraft` ends its process directly), so this is
/// allowed to go uncalled there.
#[allow(dead_code)]
pub fn windows() -> usize {
    WINDOWS.load(Ordering::Relaxed)
}

/// Count a window: the returned guard *is* the registration, and dropping it (when `android_main`
/// returns) uncounts it.
pub fn counted_window() -> Window {
    WINDOWS.fetch_add(1, Ordering::Relaxed);
    Window
}

/// One running window; see [`counted_window`].
pub struct Window;

impl Drop for Window {
    fn drop(&mut self) {
        WINDOWS.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Write `bytes` to `path` through a temporary file of its own, then rename it into place.
///
/// The unique name is the process id plus a counter: two windows are two threads of one process,
/// so a name based on either alone would collide.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    static N: AtomicUsize = AtomicUsize::new(0);
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "prefs".into());
    let tmp = path.with_file_name(format!(".{name}.{}-{}.tmp", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let mut file = std::fs::File::create(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all().ok();
    drop(file);
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}
