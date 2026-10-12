#!/usr/bin/env bash
# The Android cross-compile environment this box needs, sourced by the build and probe scripts.
#
# Two things have to be right for a Rust GUI app of this suite to cross-compile here:
#
#  * the linker for the target triple, because rustc links a NativeActivity cdylib, and
#  * the C toolchain, because PdfCraft's `pdfcraft-sign` builds `aws-lc-sys` (a C crypto library
#    behind aws-lc-rs) for every target that is not wasm32. `cc` looked for a bare `clang.exe`,
#    which is why the NDK's bin directory goes on PATH here and CC/AR/CXX name the NDK wrappers.
#
# NDK r27, minSdk 26 to match [package.metadata.android.sdk] in each shell's Cargo.toml: the
# wrapper is aarch64-linux-android26-clang, and a mismatch is a link error about the API level.
export PATH="$HOME/.cargo/bin:/c/Users/Admin/android-tools/jdk21/bin:$PATH"
export JAVA_HOME="C:\\Users\\Admin\\android-tools\\jdk21"
export ANDROID_HOME="C:\\Users\\Admin\\android-sdk"
export ANDROID_SDK_ROOT="$ANDROID_HOME"
export ANDROID_NDK_ROOT="$ANDROID_HOME\\ndk\\27.2.12479018"
export ANDROID_NDK_HOME="$ANDROID_NDK_ROOT"

NDK_BIN="/c/Users/Admin/android-sdk/ndk/27.2.12479018/toolchains/llvm/prebuilt/windows-x86_64/bin"
export PATH="$NDK_BIN:$PATH"

export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="aarch64-linux-android26-clang.cmd"
export CC_aarch64_linux_android="aarch64-linux-android26-clang.cmd"
export CXX_aarch64_linux_android="aarch64-linux-android26-clang++.cmd"
export AR_aarch64_linux_android="llvm-ar.exe"
export RANLIB_aarch64_linux_android="llvm-ranlib.exe"

# The same for x86_64, which is what the emulator runs (an x86_64 guest on this x86_64 host is the
# only one WHPX can accelerate; an arm64 guest would fall back to emulating every instruction).
# `export CC_x86_64_linux_android=...` is a *different* variable from the aarch64 one, so both are
# here and the target decides which the tools read. Sideload APKs stay arm64-only; the x86_64
# builds exist to be run in the emulator (`--target x86_64-linux-android`).
export CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER="x86_64-linux-android26-clang.cmd"
export CC_x86_64_linux_android="x86_64-linux-android26-clang.cmd"
export CXX_x86_64_linux_android="x86_64-linux-android26-clang++.cmd"
export AR_x86_64_linux_android="llvm-ar.exe"
export RANLIB_x86_64_linux_android="llvm-ranlib.exe"

# cc-rs also looks at the bare CC/AR when building a *host* helper (aws-lc's generators), and
# falls back to whatever `clang` it finds; the NDK's clang.exe on PATH above is that fallback.
