# WordCraft for Android (arm64-v8a)

An unofficial Android port of **[WordCraft](https://github.com/storytold/wordcraft)** — the real engine
and egui interface, packaged as a NativeActivity APK for sideloading on arm64 devices.

| | |
|---|---|
| Upstream project | [`storytold/wordcraft`](https://github.com/storytold/wordcraft) — Apache-2.0 OR MIT |
| Pinned commit | `2ffba93` (upstream version `0.5.0`) |
| Android package | `ai.storyteller.wordcraft` |
| Published APK | `wordcraft-0.5.0-arm64-v8a-v4.apk` — 19.9 MB, arm64-v8a, signed |
| APK sha256 | `726b570c652a9630cdc0cdc499e9157890ced6838ffdcae68937f2417e1fccfa` |
| Upstream files changed | 1 file (`crates/ui-egui/src/lib.rs`) — see [`patches/`](patches/) |
| New port crate | `apps/wordcraft-android/` — 1092 lines of Rust plus resources |

## What this repository is

This is **not a fork** of WordCraft. It holds only the Android layer, so it stays small, readable,
and obviously separable from upstream:

```
apps/wordcraft-android/     the new shell crate — copy it into your checkout of upstream
patches/                the diff against upstream — apply it after checking out the pin
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

# any keystore of your own signs an installable APK; a throwaway test key is fine for sideloading
keytool -genkeypair -keystore ks.jks -alias wordcraft -keyalg RSA -keysize 2048 -validity 10000 \
        -storepass android -keypass android -dname "CN=Local Test, O=Local, C=US"
export CARGO_APK_RELEASE_KEYSTORE="$PWD/ks.jks" CARGO_APK_RELEASE_KEYSTORE_PASSWORD=android
cargo apk build --release -p wordcraft-android
```

`scripts/android-env.sh` is the cross-compile environment this port was built with (the NDK linker
and the C compiler the native dependencies need), `scripts/build-apk.sh` wraps the build, and
`scripts/verify-apk.sh` checks the artifact: `aapt2 dump badging`, `apksigner verify`, and `llvm-nm`
on the `.so` inside the APK.

## Install

```bash
adb install -r wordcraft-0.5.0-arm64-v8a-v4.apk
```
or copy the APK to the device and open it (installing from unknown sources must be allowed).

On first run, grant **Settings ▸ Apps ▸ WordCraft ▸ Special access ▸ All files access** if you want
the in-app picker to reach files outside the app's own folders. Without it, Android 10+ refuses a
`read_dir` of shared storage by path, and the picker can only offer the app's own directory.

## Verification status

Everything below was checked on the machine this port was published from, in October 2026.
A tick means the check was *run*, not inferred; the wording says how far it goes.

| Claim | How it was checked | Result |
|---|---|---|
| `patches/…patch` applies to a clean checkout of the pin | fresh `git init` + `git fetch --depth 1 origin 2ffba93b5189410a415970be179dcf481092d756` + `git apply --check` | ✅ applies cleanly |
| The assembled tree equals the tree the APK was built from | file-by-file SHA-256 (line endings normalised) over every patched file and every file of the port crate | ✅ 12 files identical, none differing |
| The port crate compiles for Android | `cargo check -p wordcraft-android --target aarch64-linux-android`, run in a tree assembled from scratch (fresh `git fetch` of the pin, patch applied, port crate copied in) | ✅ exit 0 |
| APK is signed and is the right artifact | `apksigner verify --verbose`, `aapt2 dump badging`, sha256 | ✅ v2/v3 signatures true, `native-code: 'arm64-v8a'`, package `ai.storyteller.wordcraft` |
| Behaviour on real hardware | hand-testing on the porter's arm64 tablet | ⚠️ the fixes below were found and driven by that testing; each published binary is not individually re-verified here |

**Before trusting the APK:** the source side is verified as listed above; runtime behaviour only as far
as [`docs/PORT-NOTES.md`](docs/PORT-NOTES.md) says per fix, plus the tablet testing that produced those
fixes. There is no automated device test in this repository.

## Known gaps

- **Runtime verification is hand-testing only**, plus the source-side checks in the README. No
  automated device test.
- **All files access is a manual grant.** Until it is given, the picker can only offer the app's own
  directory (and says so on the locked root).
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
