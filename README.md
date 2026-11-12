# WordCraft for Android (arm64-v8a)

An unofficial Android port of **[WordCraft](https://github.com/storytold/wordcraft)** — the real engine
and egui interface, packaged as a NativeActivity APK for sideloading on arm64 devices.

| | |
|---|---|
| Upstream project | [`storytold/wordcraft`](https://github.com/storytold/wordcraft) — Apache-2.0 OR MIT |
| Pinned commit | `2ffba93` (upstream version `0.5.0`) |
| Android package | `ai.storyteller.wordcraft` |
| Local fixed APK (2026-11-12) | `wordcraft-0.5.0-arm64-v8a-files-ru3.apk` — 20,845,436 bytes; version `0.5.0-files-ru.3`, code `16778499`; same certificate as the previous local APK |
| Local APK sha256 | `472058a75977acca728e07ea061afcdf6181a903f5fc477f0368c888b81437ea` |
| Published APK | `wordcraft-0.5.0-arm64-v8a-20261013b.apk` — 19.9 MB, arm64-v8a, signed (release **v0.1.2**, 2026-10-13) |
| APK sha256 | `05065f17c115a5fed20f9ed6f04dcd1cd6b31bd8128e8177031e8d6f9a522ca5` |
| Upstream files changed | 2 files (`Cargo.toml`, `crates/ui-egui/src/lib.rs`) — see [`patches/`](patches/) |
| Vendored crates | `vendor/winit/` — required by the manifest patch, copied in with the crate |
| New port crate | `apps/wordcraft-android/` — 1943 lines of Rust plus resources |

## What this repository is

This is **not a fork** of WordCraft. It holds only the Android layer, so it stays small, readable,
and obviously separable from upstream:

```
apps/wordcraft-android/     the new shell crate — copy it into your checkout of upstream
patches/                the diff against upstream — apply it after checking out the pin
vendor/winit/           the patched winit the manifest points at ([patch.crates-io]) — copy it in too
docs/PORT-NOTES.md      what was changed and why, fix by fix, with how each one was checked
scripts/                cross-compile environment, build and verify scripts (from shared-android-port)
CHANGELOG.md            what is in the published APK
```

The Android layer is not a reimplementation. It is a shell crate that fills the same `Services`
struct the desktop and web shells fill, so the real engine and the real egui UI run unchanged.

## Build it

Prerequisites: Rust (rustup) with the `aarch64-linux-android` target, Android SDK + NDK r27, a
JDK 21, and `cargo install cargo-apk`.

```bash
git clone https://github.com/storytold/wordcraft.git
cd wordcraft
git fetch --depth 1 origin 2ffba93b5189410a415970be179dcf481092d756
git checkout FETCH_HEAD                        # the exact commit this port targets
git apply -3 /path/to/0001-wordcraft-android-port.patch
cp -r /path/to/wordcraft-android apps/             # the shell crate from this repository
cp -r /path/to/vendor .                        # the patched winit the manifest points at

# any keystore of your own signs an installable APK; a throwaway test key is fine for sideloading
keytool -genkeypair -keystore ks.jks -alias wordcraft -keyalg RSA -keysize 2048 -validity 10000 \
        -storepass android -keypass android -dname "CN=Local Test, O=Local, C=US"
export CARGO_APK_RELEASE_KEYSTORE="$PWD/ks.jks" CARGO_APK_RELEASE_KEYSTORE_PASSWORD=android
cargo apk build --release -p wordcraft-android  # native build stage only; complete with build-apk.ps1 below
```

`scripts/android-env.sh` is the cross-compile environment this port was built with (the NDK linker
and the C compiler the native dependencies need), `scripts/build-apk.sh` wraps the build, and
`scripts/verify-apk.sh` checks the artifact: `aapt2 dump badging`, `apksigner verify`, and `llvm-nm`
on the `.so` inside the APK.

## Install

```bash
adb install -r wordcraft-0.5.0-arm64-v8a-files-ru3.apk
```
or copy the APK to the device and open it (installing from unknown sources must be allowed).

Open and Save As both use the built-in file picker, with Russian labels when the editor uses
Russian. On first use the app opens Android's storage permission UI on the main thread: all-files
access settings on Android 11+, or read/write permission on Android 8–10. Enable access and return
to WordCraft; the picker moves to shared Documents/Download. It also offers the internal-storage
root, available SD cards and a Show all files checkbox. Opening through this picker uses the
original file path, so ordinary Save updates the original document.

WordCraft is also registered in Android's **Open with / Открыть с помощью** chooser for Word,
ODT, RTF, text, Markdown, HTML, TeX and JSON files, plus generic binary documents. The Activity
handles both cold starts and new intents delivered to an existing window. Content-provider URIs
are imported into durable private storage on a worker; use Save As to save an externally supplied
copy to a public folder. Readable file URIs are opened directly. Incoming files use the editor's
normal command path, including the unsaved-changes prompt.

**Build this version with the Windows script**, using the assembled upstream workspace described
above. A bare `cargo apk build` does not include the required Java Activity:

```powershell
./scripts/build-apk.ps1 -Workspace C:/path/to/assembled/wordcraft `
    -Sdk C:/Android/sdk -Keystore C:/path/to/wordcraft-sideload.keystore
```

Requires Python 3 and a JDK providing javac on PATH. The script builds Rust, compiles the Activity
with javac and D8, packages classes.dex and the Android 10 storage flag, aligns and signs the APK.
The default output is `wordcraft-0.5.0-arm64-v8a-files-ru3.apk` in this repository.
The keystore password expected by the script is `android`.

## Verification status

For the local fixed APK on 2026-11-12: all 12 shell tests passed, including Russian dialog
rendering and Unicode/uppercase document filenames. The final native library and classes.dex
match the build outputs byte-for-byte; Rust source/manifest files match the build workspace.
Checked APK signatures v2/v3, signing certificate continuity, arm64 ABI, native entry points,
Activity DEX methods, `hasCode`, Android 10 storage flag and final ZIP alignment. The user
reported the final APK working on their Android device. This is manual user confirmation;
there is no automated device test or recorded Android version/device model.

Everything below was checked on the machine this port was published from, in October 2026, except the
row that gives its own date.
A tick means the check was *run*, not inferred; the wording says how far it goes.

| Claim | How it was checked | Result |
|---|---|---|
| `patches/…patch` applies to a clean checkout of the pin | fresh `git init` + `git fetch --depth 1 origin 2ffba93b5189410a415970be179dcf481092d756` + `git apply --check` | ✅ applies cleanly |
| The assembled tree equals the tree the APK was built from | file-by-file SHA-256 (line endings normalised) over every patched file and every file of the port crate | ✅ 12 files identical, none differing |
| The port crate compiles for Android | `cargo check -p wordcraft-android --target aarch64-linux-android`, run in a tree assembled from scratch (fresh `git fetch` of the pin, patch applied, port crate copied in) | ✅ exit 0 |
| The port crate's own tests | `cargo test -p wordcraft-android` in the assembled tree | ✅ 12/12 pass (2026-11-12, current local build) |
| APK is signed and is the right artifact | `apksigner verify --verbose`, `aapt2 dump badging`, sha256 | ✅ v2/v3 signatures true, `native-code: 'arm64-v8a'`, package `ai.storyteller.wordcraft` |
| Behaviour on real hardware | hand-testing on the porter's arm64 tablet | ⚠️ the fixes below were found and driven by that testing; each published binary is not individually re-verified here |

**Before trusting the APK:** the source side is verified as listed above; runtime behaviour only as far
as [`docs/PORT-NOTES.md`](docs/PORT-NOTES.md) says per fix, plus the tablet testing that produced those
fixes. There is no automated device test in this repository.

## Known gaps

- **Runtime verification is hand-testing only**, plus the source-side checks in the README. No
  automated device test.
- **The built-in picker uses a special-access grant.** The app asks for it by opening the system screen
  the first time Open or Save As appears (and again from the picker's banner or a padlocked folder),
  but the permission itself has no runtime dialog, so the user still has to flip the switch there.
- **A second window is a second Activity**, so it costs a second render surface and its own memory; on
  a low-RAM tablet two heavy documents at once are the tablet's problem, not the app's.
- **No audio** — `Services::audio_out` stays `None` (`cpal` has no Android backend here).
- **Android 16 ignores the status-bar opt-out** used by theme variant A; the port ships variant B.


## The rest of this set

- [photocraft-android](https://github.com/kanaaka/photocraft-android) — PhotoCraft 0.7.0
- [vectorcraft-android](https://github.com/kanaaka/vectorcraft-android) — VectorCraft 0.9.0
- [pdfcraft-android](https://github.com/kanaaka/pdfcraft-android) — PdfCraft 0.6.0
- [gridcraft-android](https://github.com/kanaaka/gridcraft-android) — GridCraft 0.5.0
- [deckcraft-android](https://github.com/kanaaka/deckcraft-android) — DeckCraft 0.5.0
- [shared-android-port](https://github.com/kanaaka/shared-android-port) — build/verify scripts + the cross-compile environment

## Credits and licence

WordCraft is © its authors at [`storytold/wordcraft`](https://github.com/storytold/wordcraft) and is
dual-licensed Apache-2.0 / MIT; both licence texts are kept here. The Android layer in this
repository is an unofficial modification by [@kanaaka](https://github.com/kanaaka), carrying the
same licence. Not affiliated with or endorsed by the upstream project. See
[`ATTRIBUTION.md`](ATTRIBUTION.md).
