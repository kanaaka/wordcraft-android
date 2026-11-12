# Built-in file access and Open with — 2026-11-12

Open, Save As, Insert Picture and recipient selection use the built-in egui file
picker; it is not replaced by Android's system document chooser.
The picker is translated into Russian using the editor language, checks the actual
storage grant (a successful read_dir can still hide shared documents), refreshes
grants/revocations, and requests permission through the Java Activity's UI thread.
After granting access it navigates to shared Documents/Download. Storage roots
follow the actual Android user/profile, and removable volumes are listed separately.
A Show all files checkbox lets users inspect files hidden by the extension filter.
Opening a document through this picker passes its original path to the engine.

The manifest advertises ACTION_VIEW plus CATEGORY_DEFAULT for Word/docx/docm/dotx,
ODT, RTF, plain text, Markdown, HTML, TeX, JSON and application/octet-stream.
MIME-only filters match content and file URI schemes, without claiming web links.
The Java NativeActivity handles onCreate and onNewIntent (singleTop); it opens
readable file URIs directly and imports content URIs through ContentResolver while
the incoming temporary read grant is valid. Imports run on an ordered worker and
preserve filenames/extensions, including Unicode. Metadata failures fall back to
the URI name/MIME type. Each Activity owns its own queue of imported paths.

Rust polls that Activity's queue and runs the normal file.open user command after
other dialogs close. This preserves save-changes and password prompts. Content
URIs produce a private copy; use Save As to choose a shared destination. Files
opened through the built-in picker remain original filesystem documents.

The Windows packaging script includes the Java DEX, Android 10 legacy-storage
opt-out, alignment and signing. VersionName is 0.5.0-files-ru.3 and versionCode is
16778499, higher than both previous local builds. The output is
wordcraft-0.5.0-arm64-v8a-files-ru3.apk.

All 12 host tests passed, including Russian Open and Save dialog rendering,
Unicode/uppercase filenames, secondary-user storage roots and preserving unsaved
work when an external document arrives. The user reported the final APK working
on their Android device. This is manual confirmation, with no device model or
Android version recorded; there is no automated device test.
Final APK verification and SHA-256 are recorded in README.md and the checksum file.

Final artifact verification: versionCode 16778499, arm64-v8a, Android 8+; signatures
v2/v3 verified, ZIP alignment valid, signing certificate unchanged. ACTION_VIEW,
CATEGORY_DEFAULT and the required document MIME types are present in the packaged
manifest. Both onCreate and onNewIntent and the per-Activity inbox method are
present in classes.dex. The removed system-picker launch method is absent.
Packaged native code and DEX match final build outputs, and the eight Rust
source/manifest files match the assembled build workspace byte-for-byte.

APK: wordcraft-0.5.0-arm64-v8a-files-ru3.apk (20,845,436 bytes)
SHA-256: 472058a75977acca728e07ea061afcdf6181a903f5fc477f0368c888b81437ea
