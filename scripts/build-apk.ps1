param(
    [Parameter(Mandatory=$true)][string]$Workspace,
    [string]$Sdk = 'C:\Android\sdk',
    [Parameter(Mandatory=$true)][string]$Keystore,
    [string]$Output = 'wordcraft-0.5.0-arm64-v8a-files-ru3.apk',
    [switch]$RunTests
)
$ErrorActionPreference = 'Stop'
$portRoot = Split-Path $PSScriptRoot -Parent
$Workspace = (Resolve-Path -LiteralPath $Workspace).Path
if (-not (Test-Path -LiteralPath "$Workspace\crates\engine\Cargo.toml")) {
    throw 'Workspace must be the assembled, patched upstream WordCraft workspace.'
}
$env:ANDROID_HOME = $Sdk
$env:ANDROID_NDK_HOME = "$Sdk\ndk\27.2.12479018"
$env:ANDROID_NDK_ROOT = $env:ANDROID_NDK_HOME
$env:JAVA_HOME = Split-Path (Split-Path (Get-Command javac.exe).Source -Parent) -Parent
$ndkBin = "$env:ANDROID_NDK_HOME\toolchains\llvm\prebuilt\windows-x86_64\bin"
$env:PATH = "$ndkBin;$env:JAVA_HOME\bin;$env:PATH"
$env:CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER = "$ndkBin\aarch64-linux-android26-clang.cmd"
$env:CC_aarch64_linux_android = $env:CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER
$env:CXX_aarch64_linux_android = "$ndkBin\aarch64-linux-android26-clang++.cmd"
$env:AR_aarch64_linux_android = "$ndkBin\llvm-ar.exe"
$env:CARGO_APK_RELEASE_KEYSTORE = $Keystore
$env:CARGO_APK_RELEASE_KEYSTORE_PASSWORD = 'android'

Copy-Item -LiteralPath "$portRoot\apps\wordcraft-android\Cargo.toml" -Destination "$Workspace\apps\wordcraft-android\Cargo.toml" -Force
Copy-Item -Path "$portRoot\apps\wordcraft-android\src\*.rs" -Destination "$Workspace\apps\wordcraft-android\src" -Force
Push-Location $Workspace
try {
    if ($RunTests) {
        & cargo test -p wordcraft-android
        if ($LASTEXITCODE -ne 0) { throw 'Android shell tests failed' }
    }
    & cargo apk build --release -p wordcraft-android
    if ($LASTEXITCODE -ne 0) { throw 'Rust APK build failed' }
} finally { Pop-Location }

# cargo-apk packages native code only. Compile the Activity and rebuild the resource
# package with its DEX before alignment/signing. No signed APK is edited in place.
$stage = "$portRoot\target\android-bridge"
New-Item -ItemType Directory -Force -Path "$stage\classes", "$stage\dex" | Out-Null
$androidJar = "$Sdk\platforms\android-35\android.jar"
& javac --release 8 -encoding UTF-8 -classpath $androidJar -d "$stage\classes" "$portRoot\apps\wordcraft-android\java\ai\storyteller\wordcraft\WordcraftActivity.java"
if ($LASTEXITCODE -ne 0) { throw 'Activity compilation failed' }
$classFiles = @(Get-ChildItem "$stage\classes" -Recurse -Filter '*.class' | ForEach-Object FullName)
& "$Sdk\build-tools\35.0.0\d8.bat" --min-api 26 --lib $androidJar --output "$stage\dex" @classFiles
if ($LASTEXITCODE -ne 0) { throw 'DEX compilation failed' }
& python "$PSScriptRoot\package-bridge.py" --workspace $Workspace --port $portRoot --stage $stage --sdk $Sdk
if ($LASTEXITCODE -ne 0) { throw 'APK packaging failed' }
& "$Sdk\build-tools\35.0.0\zipalign.exe" -f -P 16 4 "$stage\unsigned.apk" "$stage\aligned.apk"
if ($LASTEXITCODE -ne 0) { throw 'APK alignment failed' }
$outputPath = Join-Path $portRoot $Output
& "$Sdk\build-tools\35.0.0\apksigner.bat" sign --ks $Keystore --ks-pass pass:android --out $outputPath "$stage\aligned.apk"
if ($LASTEXITCODE -ne 0) { throw 'APK signing failed' }
& "$Sdk\build-tools\35.0.0\apksigner.bat" verify --verbose $outputPath
if ($LASTEXITCODE -ne 0) { throw 'APK signature verification failed' }
& "$Sdk\build-tools\35.0.0\aapt.exe" dump badging $outputPath
if ($LASTEXITCODE -ne 0) { throw 'APK manifest verification failed' }
Get-FileHash -Algorithm SHA256 -LiteralPath $outputPath
