//! An in-app file picker for Android, drawn with egui — no Java, no native dialog.
//!
//! # Why this exists
//!
//! `rfd` has no Android backend (its `backend.rs` compiles no module at all for a
//! target that is neither Linux/BSD, macOS, Windows nor wasm32), and Android's
//! own picker — the Storage Access Framework — needs `startActivityForResult`,
//! which `android-activity` 0.6 does not expose. So a ported desktop app has no
//! file dialogs at all: Open and Save As are silently treated as cancelled.
//! This module draws the picker instead.
//!
//! # How it plugs in
//!
//! Every app in this suite has a host-services seam that is *asynchronous* on
//! the platforms that can't block the frame — the browser build uses it, and so
//! does this. [`request`] queues a dialog and returns a [`Receiver`] the app
//! polls once a frame; [`draw`] answers on it from a later frame. The app's own
//! follow-up (open the file, save there and carry on) then runs, unchanged.
//!
//! Shape per app:
//!
//! - `Services::file_dialog(req) -> Receiver<Option<String>>` (WordCraft, this
//!   crate's `FileDialogRequest`): answer `Some(path)`, or `None` for cancelled.
//! - `Services::pick_save_async(name) -> Receiver<Option<String>>` (GridCraft,
//!   DeckCraft): the same, for Save As and the exports. Without it the app is
//!   handed only finished bytes and never learns where they went.
//! - `Services::open_async` + `Services::inbox` and `pick_picture_async(inbox)`
//!   (GridCraft, DeckCraft — the web seam): the shell polls the receiver, reads
//!   the file and pushes `(name, bytes)` into the inbox.
//! - `download(name, bytes)` (GridCraft, DeckCraft — the last resort): ask for a
//!   path, then write the bytes there.
//! - `rfd::AsyncFileDialog` (PdfCraft): the crate's own `crate::rfd` shim, which
//!   registers here and hands back a future the picker's answer completes.
//!
//! An empty answer means cancelled, which is what each of those seams reads as
//! "the user backed out". Never drop the sender instead.
//!
//! # One dialog per window, not per process
//!
//! The dialog in flight is a **thread-local**, not a global. A second window of
//! the same app is a second `android_main` on a thread of its own (a second
//! Activity, which android-activity supports), and a global would give the two
//! windows one shared dialog: the second window's request would cancel the
//! first's, both would draw it, and the file the user tapped could open in the
//! other window.
//!
//! # Permissions
//!
//! Android 10+ forbids reading `/storage/emulated/0/Download` by path. Two
//! routes exist and this picker works with either, because it probes each root
//! and only offers the ones it can actually read:
//!
//! - **App folder** — `external_data_path()`, the app's own directory. Always
//!   readable and writable, no permission at all. Files can be pushed there over
//!   USB.
//! - **Shared storage** — Download / Pictures / DCIM / Documents. Needs
//!   "All files access" (`MANAGE_EXTERNAL_STORAGE`, granted by hand in
//!   Settings ▸ Apps ▸ <App> ▸ Special access) or `READ_MEDIA_IMAGES` for
//!   reading media. All four are declared in the crate's `Cargo.toml`.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::mpsc::{Receiver, Sender, channel};

/// The app's external files directory, captured in `android_main`.
///
/// `<shared storage>/Android/data/<package>/files` — always readable and
/// writable with no permission, so it is both the picker's default root and the
/// place Save falls back to. Shared by every window of the app, which is right:
/// there is one such folder.
pub static EXT_DIR: OnceLock<PathBuf> = OnceLock::new();

const SHARED_ROOT: &str = "/storage/emulated/0";

/// Which dialog is open.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// One file to open.
    Open,
    /// Several files.
    OpenMany,
    /// A path to save to.
    Save,
    /// A folder.
    Folder,
}

/// The dialog being shown: what was asked, and how to answer it.
struct Pending {
    kind: Kind,
    title: String,
    /// Extensions the listing is filtered to (none: every file).
    extensions: Option<Vec<String>>,
    /// Directory being browsed.
    dir: PathBuf,
    /// File name for a Save dialog.
    name: String,
    /// Chosen files, for a multi-select dialog.
    picked: Vec<PathBuf>,
    /// A message to show inside the picker (usually a permission problem).
    note: Option<String>,
    /// Whether the starting directory was already applied.
    started: bool,
    /// Where the answer goes; the app polls it once a frame.
    answer: Option<Sender<Vec<PathBuf>>>,
}

thread_local! {
    /// The dialog *this* window is showing. One per thread; see the module docs.
    static PENDING: RefCell<Option<Pending>> = const { RefCell::new(None) };
}

/// Every extension a filter list offers, or `None` when it offers none (then
/// every file is listed). Convenience for the call sites that build filters.
///
/// Not every app reaches every entry point in this file; it is one file shared by four shells, so
/// an unused one is expected rather than a mistake to delete.
#[allow(dead_code)]
pub fn extensions(filters: &[&[&str]]) -> Option<Vec<String>> {
    let exts: Vec<String> = filters.iter().flat_map(|e| e.iter().map(|s| (*s).to_string())).collect();
    (!exts.is_empty()).then_some(exts)
}

/// Queue a dialog and return the receiver its answer arrives on.
///
/// A dialog already open in *this* window is answered "cancelled" first; the app
/// can only use one answer at a time, and leaving the old request waiting would
/// strand it.
pub fn request(kind: Kind, title: &str, extensions: Option<Vec<String>>, suggested: Option<String>) -> Receiver<Vec<PathBuf>> {
    let (tx, rx) = channel();
    PENDING.with(|slot| {
        let mut slot = slot.borrow_mut();
        if let Some(prev) = slot.take()
            && let Some(tx) = prev.answer
        {
            let _ = tx.send(Vec::new());
        }
        *slot = Some(Pending {
            kind,
            title: title.to_string(),
            extensions,
            dir: PathBuf::new(),
            name: suggested.unwrap_or_default(),
            picked: Vec::new(),
            note: None,
            started: false,
            answer: Some(tx),
        });
    });
    rx
}

/// Whether this window has a dialog waiting to be drawn.
#[allow(dead_code)]
pub fn is_open() -> bool {
    PENDING.with(|slot| slot.borrow().is_some())
}

/// Close this window's dialog, if any, answering "cancelled" — for a shell that
/// is quitting while one is open.
#[allow(dead_code)]
pub fn cancel() {
    PENDING.with(|slot| {
        if let Some(pending) = slot.borrow_mut().take()
            && let Some(tx) = pending.answer
        {
            let _ = tx.send(Vec::new());
        }
    });
}

/// Roots to offer, as (label, path).
fn roots() -> Vec<(String, PathBuf)> {
    let mut v = Vec::new();
    if let Some(d) = EXT_DIR.get() {
        v.push(("App folder".to_string(), d.clone()));
        // Sibling of the app's own files, still owned by the app.
        if let Some(parent) = d.parent() {
            v.push(("App storage".to_string(), parent.to_path_buf()));
        }
    }
    for (label, sub) in [("Download", "Download"), ("Pictures", "Pictures"), ("DCIM", "DCIM"), ("Documents", "Documents")] {
        v.push((label.to_string(), Path::new(SHARED_ROOT).join(sub)));
    }
    v.push(("Internal storage".to_string(), PathBuf::from(SHARED_ROOT)));
    v
}

/// The best directory to open first: the app folder is always writable, so it
/// is the only safe default — a desktop path arriving as a suggestion never
/// exists on the device.
fn default_dir() -> PathBuf {
    EXT_DIR.get().cloned().unwrap_or_else(|| PathBuf::from(SHARED_ROOT))
}

fn readable(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok()
}

/// Directory entries, folders first, filtered by `extensions` when given.
fn entries(dir: &Path, extensions: Option<&Vec<String>>) -> Result<(Vec<PathBuf>, Vec<PathBuf>), String> {
    let rd = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let (mut dirs, mut files) = (Vec::new(), Vec::new());
    for entry in rd.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            dirs.push(path);
        } else if let Some(exts) = extensions {
            let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase());
            if ext.as_ref().is_some_and(|e| exts.iter().any(|w| w.eq_ignore_ascii_case(e))) {
                files.push(path);
            }
        } else {
            files.push(path);
        }
    }
    dirs.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()));
    files.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()));
    Ok((dirs, files))
}

fn file_label(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

/// Draw the picker if this window has a dialog queued. Called from the shell's
/// `ui`, *after* forwarding to the app's own `ui`, so the window floats above
/// the editor.
pub fn draw(ctx: &egui::Context) {
    // Taken out for the duration rather than borrowed, so nothing in here can trip over a
    // re-entrant borrow; it is put back below while the user has not answered.
    let Some(mut pending) = PENDING.with(|slot| slot.borrow_mut().take()) else {
        return;
    };
    let kind = pending.kind;
    // First frame: pick a starting directory.
    if !pending.started {
        pending.started = true;
        pending.dir = default_dir();
        if kind == Kind::Save && pending.name.is_empty() {
            pending.name = "untitled".to_string();
        }
    }

    let mut answer: Option<Vec<PathBuf>> = None;
    let mut cancel = false;
    let mut navigate: Option<PathBuf> = None;

    let dir = pending.dir.clone();
    let listing = entries(&dir, pending.extensions.as_ref());
    let multi = kind == Kind::OpenMany;
    // A tablet screen is smaller than the desktop default, and in portrait
    // narrower than a hard-coded desktop width. `content_rect` is the area egui
    // considers safe (it excludes an OS status bar where the backend reports one).
    let screen = ctx.content_rect();
    let (w, h) = ((screen.width() * 0.92).min(720.0), (screen.height() * 0.82).min(620.0));
    let title = pending.title.clone();

    egui::Window::new(title)
        .collapsible(false)
        .resizable(true)
        .default_width(w)
        .default_height(h)
        .default_pos(screen.center() - egui::vec2(w, h) / 2.0)
        .show(ctx, |ui| {
            // Finger-sized rows: the default spacing is sized for a mouse.
            ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
            ui.spacing_mut().button_padding = egui::vec2(10.0, 8.0);

            // Roots
            ui.horizontal_wrapped(|ui| {
                for (label, path) in roots() {
                    let ok = readable(&path);
                    let here = path == dir;
                    let text = if ok { label } else { format!("{label} \u{1F512}") };
                    if ui.add_enabled(ok && !here, egui::Button::new(text)).clicked() {
                        navigate = Some(path);
                    }
                }
            });
            ui.separator();

            // Current path + up
            ui.horizontal_wrapped(|ui| {
                if ui.button("\u{2B06} Up").clicked()
                    && let Some(parent) = dir.parent()
                {
                    navigate = Some(parent.to_path_buf());
                }
                if ui.button("\u{1F4C1} App folder").clicked() {
                    navigate = Some(default_dir());
                }
                ui.label(egui::RichText::new(dir.display().to_string()).monospace().small());
            });

            if let Some(note) = &pending.note {
                ui.colored_label(egui::Color32::from_rgb(220, 160, 60), note);
            }

            ui.separator();
            let list_h = (h - 190.0).max(120.0);
            egui::ScrollArea::vertical().max_height(list_h).show(ui, |ui| match &listing {
                Err(e) => {
                    ui.label("Cannot read this folder.");
                    ui.label(egui::RichText::new(e.clone()).monospace().weak().small());
                    ui.label("Grant \"All files access\" in Settings \u{25B8} Apps \u{25B8} Special access to browse shared storage.");
                }
                Ok((dirs, files)) => {
                    if dirs.is_empty() && files.is_empty() {
                        ui.label("(empty \u{2014} no matching files)");
                    }
                    for d in dirs {
                        if ui.button(format!("\u{1F4C1} {}", file_label(d))).clicked() {
                            navigate = Some(d.clone());
                        }
                    }
                    if kind != Kind::Folder {
                        for f in files {
                            let chosen = pending.picked.iter().any(|p| p == f);
                            let mark = if chosen { "\u{2611}" } else { "\u{2610}" };
                            let label = if multi { format!("{mark} {}", file_label(f)) } else { format!("   {}", file_label(f)) };
                            if ui.button(label).clicked() {
                                if kind == Kind::Save {
                                    pending.name = file_label(f);
                                } else if multi {
                                    if chosen {
                                        pending.picked.retain(|p| p != f);
                                    } else {
                                        pending.picked.push(f.clone());
                                    }
                                } else {
                                    answer = Some(vec![f.clone()]);
                                }
                            }
                        }
                    }
                }
            });

            ui.separator();
            ui.horizontal_wrapped(|ui| match kind {
                Kind::Save => {
                    ui.label("File name:");
                    ui.add(egui::TextEdit::singleline(&mut pending.name).desired_width((w - 260.0).max(160.0)));
                    if ui.button("Save").clicked() {
                        let name = pending.name.trim().to_string();
                        if name.is_empty() {
                            pending.note = Some("Enter a file name.".to_string());
                        } else {
                            answer = Some(vec![dir.join(name)]);
                        }
                    }
                }
                Kind::Folder => {
                    if ui.button("Choose this folder").clicked() {
                        answer = Some(vec![dir.clone()]);
                    }
                }
                Kind::OpenMany => {
                    let n = pending.picked.len();
                    let label = if n == 0 { "Place".to_string() } else { format!("Place {n} file(s)") };
                    if ui.add_enabled(n > 0, egui::Button::new(label)).clicked() {
                        answer = Some(pending.picked.clone());
                    }
                }
                Kind::Open => {
                    if ui.button("Open").clicked() {
                        // A file has to be tapped in the list above for an Open dialog.
                        pending.note = Some("Tap a file to open it.".to_string());
                    }
                }
            });
            if ui.button("Cancel").clicked() {
                cancel = true;
            }

            if dir == default_dir() {
                ui.label(egui::RichText::new("Tip: push files here over USB, or grant \"All files access\" to browse the whole tablet.").weak().small());
            }
        });

    if let Some(dir) = navigate {
        pending.dir = dir;
        pending.note = None;
    }
    if answer.is_some() || cancel {
        // Cancelled: an empty answer is what every seam here reads as one.
        let paths = answer.unwrap_or_default();
        if let Some(tx) = pending.answer.take() {
            let _ = tx.send(paths);
        }
    } else {
        // Still open: it stays this window's dialog. The app only wakes the window about once a
        // second while it waits, so the picker keeps asking for frames itself.
        PENDING.with(|slot| *slot.borrow_mut() = Some(pending));
        ctx.request_repaint();
    }
}
