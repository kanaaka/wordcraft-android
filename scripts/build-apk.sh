#!/usr/bin/env bash
# Build and sign one Craft-app Android APK.
#
#   usage: build-apk.sh <repo> <crate> [features...]
#   e.g.   build-apk.sh wordcraft wordcraft-android
#
# Environment this box needs (Rust 1.99 via rustup, no PATH entries by default):
#   * $HOME/.cargo/bin                — cargo, cargo-apk
#   * the Android SDK + NDK r27.2     — ANDROID_HOME and friends
#   * the Temurin JDK 21              — apksigner runs on it
# and each app signs with its own sideload keystore under android-tools/ (alias = the app
# name, store and key password `android`), so `cargo apk build --release` produces a signed
# APK rather than a debug-signed one.
set -o pipefail
repo="$1"
crate="$2"
shift 2

# One place for the toolchain environment: rustup's bin, the JDK, the SDK/NDK, and the C
# cross-compiler aws-lc-sys needs. That file says why each piece is there.
. "$HOME/projects/shared-android-port/android-env.sh"

cd "$HOME/projects/$repo" || exit 1
echo "=== building $crate in $repo ($(date)) ==="
CARGO_APK_RELEASE_KEYSTORE="C:/Users/Admin/android-tools/$repo-sideload.keystore" \
CARGO_APK_RELEASE_KEYSTORE_PASSWORD=android \
cargo apk build --release -p "$crate" "$@"
status=$?
echo "=== $crate exit=$status ($(date)) ==="
exit $status
