#!/usr/bin/env bash
# Verify a built Craft-app APK the way the skill says to: never trust `exit=0`.
#
#   usage: verify-apk.sh <repo> <apk-name> <lib-name>
set -o pipefail
export PATH="/c/Users/Admin/android-tools/jdk21/bin:$PATH"
BT="C:/Users/Admin/android-sdk/build-tools/35.0.0"
NDK_BIN="C:/Users/Admin/android-sdk/ndk/27.2.12479018/toolchains/llvm/prebuilt/windows-x86_64/bin"
PLATFORM="C:/Users/Admin/android-sdk/platforms/android-35/android.jar"

repo="$1"; apk_name="$2"; lib="$3"
apk="C:/Users/Admin/projects/$repo/target/release/apk/$apk_name.apk"
so_dir="/c/Users/Admin/projects/$repo/target/aarch64-linux-android/release"

echo "######## $apk"
"$BT/aapt2.exe" dump badging "$apk" 2>/dev/null | grep -E "^package|^sdkVersion|^targetSdkVersion|^application|native-code|^launchable-activity|uses-permission|^feature"
echo "---- signature:"
"$BT/apksigner.bat" verify --verbose "$apk" 2>&1 | grep -E "Verified using|Verifies|DOES NOT VERIFY"
echo "---- native symbols in lib$lib.so (expect android_main + ANativeActivity_onCreate):"
# llvm-nm is a native binary: an /c/... path would not be found (MSYS path translation is off here).
SO="C:/Users/Admin/projects/$repo/target/aarch64-linux-android/release/lib$lib.so"
if [ -f "$(cygpath -u "$SO" 2>/dev/null || echo "$SO")" ]; then
  "$NDK_BIN/llvm-nm.exe" --dynamic --defined-only "$SO" 2>/dev/null | grep -E "android_main|ANativeActivity_onCreate"
else
  echo "no $SO (build did not produce one)"
fi
echo "---- .so inside the APK:"
unzip -l "$apk" 2>/dev/null | grep -oE "lib/[a-z0-9-]+/lib[a-z]*\.so" | sort -u
echo "---- launcher activity lib_name meta-data (must match the .so name above):"
"$BT/aapt2.exe" dump xmltree --file AndroidManifest.xml "$apk" 2>/dev/null | grep -E "android.app.lib_name" -A1 | grep -E "value"
echo "---- the fullscreen fix, resolved by attribute id against android-35 (not by style name):"
"$BT/aapt2.exe" dump resources "$apk" 2>/dev/null | grep -A3 "style/.*Fullscreen" | head -4
ATTR=$("$BT/aapt2.exe" dump resources "$apk" 2>/dev/null | grep -A3 "style/.*Fullscreen" | grep -oE "0x0101[0-9a-f]+" | head -1)
if [ -n "$ATTR" ]; then
  echo "  $ATTR = $("$BT/aapt2.exe" dump resources "$PLATFORM" 2>/dev/null | grep "resource $ATTR " | grep -oE "attr/[A-Za-z]+" | head -1)"
fi
