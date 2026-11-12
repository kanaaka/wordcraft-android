//! An in-app file picker for Android, drawn with egui — no Java, no native dialog.
//!
//! # Why this exists
//!
//! `rfd` has no Android backend (its `backend.rs` compiles no module at all for a
//! target that is neither Linux/BSD, macOS, Windows nor wasm32), and Android's
//! own picker is not needed here: this module supplies the built-in Open and Save As
//! dialogs, with shared-storage access requested through the Android Activity.
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
//!   "All files access" (`MANAGE_EXTERNAL_STORAGE`), which is not a runtime
//!   permission: the system only offers the toggle in
//!   Settings ▸ Apps ▸ <App> ▸ Special access. The picker therefore *asks* for it
//!   through [`crate::permissions`] — the first in-app dialog of a process opens that
//!   screen, and a locked root or the banner opens it again — instead of just
//!   drawing a padlock and leaving the user there. All the storage permissions
//!   are declared in the crate's `Cargo.toml`.
//!
//! The picker also starts Open in the user's Documents folder once it is
//! readable, because that is where the documents are; before the grant it starts
//! in the app folder, the one place that always works.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};

/// The app's external files directory, captured in `android_main`.
///
/// `<shared storage>/Android/data/<package>/files` — always readable and
/// writable with no permission, so it is both the picker's default root and the
/// place Save falls back to. Shared by every window of the app, which is right:
/// there is one such folder.
pub static EXT_DIR: OnceLock<PathBuf> = OnceLock::new();

fn storage_root_from(external_files: &Path) -> Option<&Path> {
    let android = external_files.ancestors().nth(3)?;
    (android.file_name()? == "Android").then(|| android.parent()).flatten()
}

fn shared_root() -> &'static Path {
    EXT_DIR.get().and_then(|p| storage_root_from(p)).unwrap_or_else(|| Path::new("/storage/emulated/0"))
}

/// The shared folders the picker offers, root included: the places user documents actually
/// live. Whether each one can be read is probed per frame — that is what makes the picker
/// honest about a missing grant.
fn shared_roots() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> =
        ["Download", "Pictures", "DCIM", "Documents"].iter().map(|sub| shared_root().join(sub)).collect();
    v.push(shared_root().to_path_buf());
    v
}

/// Whether shared-storage access is missing. Scoped storage can return an empty
/// directory listing instead of a permission error, so inspect the grant itself.
fn needs_access() -> bool {
    // Scoped storage may allow listing a directory but hide all of its documents.
    // A successful read_dir is therefore not proof of shared-storage access.
    !crate::permissions::granted()
}

/// Match the editor's selected interface language, including changes made in Options.
pub fn text(en: &'static str, ru: &'static str) -> &'static str {
    if wordcraft_ui_egui::i18n::current().code() == "ru" { ru } else { en }
}

/// Whether the system screen has been opened for this process already. It is opened once, when
/// the first in-app dialog appears; subsequent dialogs keep a manual access button.
static ASKED: AtomicBool = AtomicBool::new(false);

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
    /// Move to shared storage once after returning from the permission screen.
    awaiting_access: bool,
    /// Allow inspection of files whose extensions the document filter does not recognise.
    show_all: bool,
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
            awaiting_access: false,
            show_all: false,
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
        v.push((text("App folder", "Папка приложения").to_string(), d.clone()));
        // Sibling of the app's own files, still owned by the app.
        if let Some(parent) = d.parent() {
            v.push((text("App storage", "Хранилище приложения").to_string(), parent.to_path_buf()));
        }
    }
    let labels = [text("Download", "Загрузки"), text("Pictures", "Изображения"), text("DCIM", "Камера"), text("Documents", "Документы"), text("Internal storage", "Внутренняя память")];
    for (label, path) in labels.into_iter().zip(shared_roots()) {
        v.push((label.to_string(), path));
    }
    if let Ok(volumes) = std::fs::read_dir("/storage") {
        for volume in volumes.flatten() {
            let name = volume.file_name().to_string_lossy().into_owned();
            if name != "emulated" && name != "self" && volume.path().is_dir() {
                v.push((format!("{} ({name})", text("SD card", "Карта памяти")), volume.path()));
            }
        }
    }
    v
}

/// The app's folder in `Android/media`, which *is* reachable over a USB cable (`Android/data` is
/// not, from Android 11 on): `…/Android/data/<package>/files` → `…/Android/media/<package>`.
pub fn media_dir() -> Option<PathBuf> {
    media_dir_from(EXT_DIR.get()?)
}

/// The derivation, on its own so it can be pinned by a test: `…/Android/data/<package>/files`
/// → `…/Android/media/<package>`.
fn media_dir_from(external_files: &Path) -> Option<PathBuf> {
    let package_dir = external_files.parent()?;
    let android = package_dir.parent()?.parent()?;
    Some(android.join("media").join(package_dir.file_name()?))
}

/// The Fonts folder inside it: the visible folder a font file is dropped into.
pub fn fonts_dir() -> Option<PathBuf> {
    Some(media_dir()?.join("Fonts"))
}

/// Point the font scan at a folder the user can reach over a USB cable.
///
/// # Why this is needed on Android and on no other platform
///
/// `system_font_dirs()` in this app's font code takes its Linux branch here and scans
/// `/usr/share/fonts`, `$HOME/.fonts` and `$HOME/.local/share/fonts` — and none of those exist on
/// Android: the platform's own faces are in `/system/fonts`, and `HOME` is unset. So the app knows
/// only its bundled families and there is no way to add one — where VectorCraft and PhotoCraft have
/// `set_platform_font_files`/`set_app_font_dir` hooks, this app's font code has neither. It *does*
/// read `HOME` at every scan, though, so pointing `HOME` at the app's media folder and putting the
/// fonts where that branch looks gives the same result with no change to the app: drop a `.ttf`
/// into `Fonts/` over the cable, reopen the app, and it is in the font list.
///
/// The two names the scan looks for are hidden ones (`.fonts`, `.local/share/fonts`), so both are
/// symlinks to the visible `Fonts` folder — the user drops a file in one place and the app reads it
/// from the other.
///
/// The scan runs once per process (a `OnceLock` in the app), which is why a new font needs the app
/// reopened rather than a reload.
#[cfg(target_os = "android")]
pub fn register_fonts() {
    let Some(fonts) = fonts_dir() else {
        log::warn!("fonts: no Fonts folder to register; adding a font will not work");
        return;
    };
    let Some(media) = media_dir() else {
        return;
    };
    if let Err(e) = std::fs::create_dir_all(&fonts) {
        log::warn!("fonts: could not make {}: {e}", fonts.display());
        return;
    }
    log::info!("fonts: the Fonts folder is {}", fonts.display());
    // SAFETY: called from `android_main` before eframe starts, so nothing else is reading the
    // environment yet; the app's font code only ever reads `HOME`.
    unsafe { std::env::set_var("HOME", &media) };
    link_fonts_dir(&fonts, &media.join(".fonts"));
    link_fonts_dir(&fonts, &media.join(".local").join("share").join("fonts"));
}

/// Make `link` a directory symlink to `target`, replacing a stale one. Failure is logged and
/// survivable: the folder the user drops fonts into still exists, the app just cannot see it yet.
#[cfg(target_os = "android")]
fn link_fonts_dir(target: &Path, link: &Path) {
    if let Some(parent) = link.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::fs::read_link(link).is_ok() {
        let _ = std::fs::remove_file(link);
    }
    if !link.exists()
        && let Err(e) = std::os::unix::fs::symlink(target, link)
    {
        log::warn!("fonts: could not link {} → {}: {e}", link.display(), target.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_root_follows_android_user_and_external_volume() {
        for root in ["/storage/emulated/0", "/storage/emulated/10", "/storage/ABCD-1234"] {
            let files = Path::new(root).join("Android/data/ai.storyteller.wordcraft/files");
            assert_eq!(storage_root_from(&files), Some(Path::new(root)));
        }
        assert_eq!(storage_root_from(Path::new("/data/user/0/package/files")), None);
    }

    #[test]
    fn open_dialog_renders_in_russian() {
        use wordcraft_ui_egui::i18n::{Lang, set_current};
        set_current(Lang::from_pref("ru"));
        let _receiver = request(Kind::Open, text("Open file", "Открыть файл"), extensions(&[&["docx"]]), None);
        let ctx = egui::Context::default();
        let mut texts = Vec::new();
        fn collect(shape: &egui::epaint::Shape, texts: &mut Vec<String>) {
            match shape {
                egui::epaint::Shape::Text(text) => texts.push(text.galley.text().to_string()),
                egui::epaint::Shape::Vec(shapes) => for shape in shapes { collect(shape, texts); },
                _ => {}
            }
        }
        for _ in 0..3 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| draw(ui.ctx()));
            output.textures_delta.clear();
            for shape in output.shapes { collect(&shape.shape, &mut texts); }
        }
        cancel();
        set_current(Lang::EN);
        for expected in ["Открыть файл", "Открыть", "Отмена", "Показать все файлы", "Разрешить доступ к файлам"] {
            assert!(texts.iter().any(|t| t == expected), "Missing {expected}: {texts:?}");
        }
    }

    #[test]
    fn save_dialog_renders_in_russian() {
        use wordcraft_ui_egui::i18n::{Lang, set_current};
        set_current(Lang::from_pref("ru"));
        let _receiver = request(Kind::Save, text("Save As", "Сохранить как"), None, Some("test.docx".into()));
        let ctx = egui::Context::default();
        let mut visible = Vec::new();
        fn collect(shape: &egui::epaint::Shape, texts: &mut Vec<String>) {
            match shape {
                egui::epaint::Shape::Text(text) => texts.push(text.galley.text().to_string()),
                egui::epaint::Shape::Vec(shapes) => {
                    for shape in shapes { collect(shape, texts); }
                }
                _ => {}
            }
        }
        // The first egui pass measures the window; subsequent passes paint its text.
        for _ in 0..3 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| draw(ui.ctx()));
            output.textures_delta.clear(); // This test inspects text without a GPU renderer.
            for shape in output.shapes { collect(&shape.shape, &mut visible); }
        }
        cancel();
        set_current(Lang::EN);
        assert!(visible.iter().any(|s| s == "Сохранить как"), "{visible:?}");
        assert!(visible.iter().any(|s| s == "Отмена"), "{visible:?}");
        assert!(visible.iter().any(|s| s == "Разрешить доступ к файлам"), "{visible:?}");
        assert!(!visible.iter().any(|s| s == "Save" || s == "Cancel" || s == "App folder"));
    }

    #[test]
    fn listing_keeps_folders_and_matches_uppercase_document_extensions() {
        let dir = std::env::temp_dir().join(format!("wordcraft-picker-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(dir.join("subfolder")).unwrap();
        for name in ["Документ.DOCX", "notes.txt", "image.png", ".hidden.docx"] {
            std::fs::write(dir.join(name), b"test").unwrap();
        }
        let (folders, files) = entries(&dir, Some(&vec!["docx".into(), "txt".into()])).unwrap();
        assert_eq!(folders, vec![dir.join("subfolder")]);
        assert_eq!(files.len(), 2);
        assert!(files.contains(&dir.join("Документ.DOCX")));
        assert!(files.contains(&dir.join("notes.txt")));
        assert_eq!(entries(&dir, None).unwrap().1.len(), 3);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}


/// The best directory to open first: the app folder is always writable, so it
/// is the only safe default — a desktop path arriving as a suggestion never
/// exists on the device.
fn default_dir() -> PathBuf {
    EXT_DIR.get().cloned().unwrap_or_else(|| shared_root().to_path_buf())
}

/// Where an Open (or folder) dialog starts: the first user-facing folder that can actually be
/// read, so a tablet that has granted storage shows the user's documents rather than the app's
/// private folder. Save keeps [`default_dir`]: the app folder is the one place a save always
/// works, with or without a permission.
fn open_start_dir() -> PathBuf {
    ["Documents", "Download"]
        .iter()
        .map(|sub| shared_root().join(sub))
        .find(|p| readable(p))
        .unwrap_or_else(default_dir)
}

/// Open the system screen that hands out shared-storage access, and say in the picker what to
/// do there. Used by the banner, by a tap on a locked root, and once per process on the first
/// dialog.
fn ask_for_access(pending: &mut Pending) {
    let shown = crate::permissions::request();
    pending.awaiting_access = shown;
    pending.note = Some(if shown {
        text("Allow WordCraft to see the tablet's storage in the screen that just opened, then come back here.", "В открывшемся окне разрешите WordCraft доступ к файлам, затем вернитесь в приложение.").to_string()
    } else {
        text("Grant all files access in Android settings.", "Разрешите доступ ко всем файлам в настройках Android: Приложения → Специальный доступ → Доступ ко всем файлам → WordCraft.").to_string()
    });
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
        pending.dir = if kind == Kind::Save { default_dir() } else { open_start_dir() };
        if kind == Kind::Save && pending.name.is_empty() {
            pending.name = text("untitled", "Без названия").to_string();
        }
        // Both Open and Save As need path-based access to public folders.
        if needs_access() && !ASKED.swap(true, Ordering::Relaxed) {
            ask_for_access(&mut pending);
        }
    }

    let locked_out = needs_access();
    if pending.awaiting_access && !locked_out {
        pending.awaiting_access = false;
        pending.dir = open_start_dir();
        pending.note = None;
    }

    let mut answer: Option<Vec<PathBuf>> = None;
    let mut cancel = false;
    let mut navigate: Option<PathBuf> = None;

    let dir = pending.dir.clone();
    let listing = entries(&dir, if pending.show_all { None } else { pending.extensions.as_ref() });
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

            // Shared storage the app cannot read: say it where the user is already looking for
            // files, and make the fix one tap away.
            if locked_out {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        egui::RichText::new(text("Shared-storage access is required to browse documents.", "Для просмотра документов нужен доступ к файлам устройства."))
                            .color(egui::Color32::from_rgb(220, 160, 60)),
                    );
                    if ui.button(text("Allow file access", "Разрешить доступ к файлам")).clicked() {
                        ask_for_access(&mut pending);
                    }
                });
                ui.separator();
            }

            // Roots. A locked one stays tappable: tapping it is how the user asks for the
            // permission the padlock is about.
            ui.horizontal_wrapped(|ui| {
                for (label, path) in roots() {
                    let ok = readable(&path);
                    let here = path == dir;
                    let caption = if ok { label } else { format!("{label} \u{1F512}") };
                    if ui.add_enabled(!here, egui::Button::new(caption)).clicked() {
                        if ok {
                            navigate = Some(path);
                        } else if crate::permissions::granted() {
                            // The permission is there and the folder still cannot be read — a
                            // missing folder, or one this Android version refuses by path (on
                            // Android 10 scoped storage has no "All files access" to grant).
                            pending.note = Some(text("This folder is unavailable or does not exist.", "Эта папка недоступна или не существует.").to_string());
                        } else {
                            ask_for_access(&mut pending);
                        }
                    }
                }
            });
            ui.separator();

            // Current path + up
            ui.horizontal_wrapped(|ui| {
                if ui.button(text("↑ Up", "↑ Вверх")).clicked()
                    && let Some(parent) = dir.parent()
                {
                    navigate = Some(parent.to_path_buf());
                }
                if ui.button(text("App folder", "Папка приложения")).clicked() {
                    navigate = Some(default_dir());
                }
                ui.label(egui::RichText::new(dir.display().to_string()).monospace().small());
            });

            if let Some(note) = &pending.note {
                ui.colored_label(egui::Color32::from_rgb(220, 160, 60), note);
            }

            ui.separator();
            if pending.extensions.is_some() {
                ui.checkbox(&mut pending.show_all, text("Show all files", "Показать все файлы"));
            }
            let list_h = (h - 190.0).max(120.0);
            egui::ScrollArea::vertical().max_height(list_h).show(ui, |ui| match &listing {
                Err(e) => {
                    ui.label(text("Cannot read this folder.", "Не удалось прочитать папку."));
                    ui.label(egui::RichText::new(e.clone()).monospace().weak().small());
                    ui.label(text("Allow file access to browse shared storage.", "Для просмотра памяти устройства разрешите доступ к файлам."));
                }
                Ok((dirs, files)) => {
                    if dirs.is_empty() && files.is_empty() {
                        ui.label(text("(empty — no matching files)", "(нет подходящих файлов)"));
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
                    ui.label(text("File name:", "Имя файла:"));
                    ui.add(egui::TextEdit::singleline(&mut pending.name).desired_width((w - 260.0).max(160.0)));
                    if ui.button(text("Save", "Сохранить")).clicked() {
                        let name = pending.name.trim().to_string();
                        if name.is_empty() {
                            pending.note = Some(text("Enter a file name.", "Введите имя файла.").to_string());
                        } else {
                            answer = Some(vec![dir.join(name)]);
                        }
                    }
                }
                Kind::Folder => {
                    if ui.button(text("Choose this folder", "Выбрать эту папку")).clicked() {
                        answer = Some(vec![dir.clone()]);
                    }
                }
                Kind::OpenMany => {
                    let n = pending.picked.len();
                    let label = if n == 0 { text("Place", "Добавить").to_string() } else { format!("{}: {n}", text("Place files", "Добавить файлов")) };
                    if ui.add_enabled(n > 0, egui::Button::new(label)).clicked() {
                        answer = Some(pending.picked.clone());
                    }
                }
                Kind::Open => {
                    if ui.button(text("Open", "Открыть")).clicked() {
                        // A file has to be tapped in the list above for an Open dialog.
                        pending.note = Some(text("Tap a file to open it.", "Нажмите на файл, чтобы открыть его.").to_string());
                    }
                }
            });
            if ui.button(text("Cancel", "Отмена")).clicked() {
                cancel = true;
            }

            if dir == default_dir() {
                ui.label(egui::RichText::new(text("Allow file access to save to Documents or Download.", "Разрешите доступ к файлам, чтобы сохранять в папки «Документы» и «Загрузки».")).weak().small());
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
