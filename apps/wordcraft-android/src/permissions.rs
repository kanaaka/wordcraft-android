//! Shared-storage access: asking the system for it, and knowing whether the app has it.
//!
//! # Why the app has to ask at all
//!
//! The picker ([`crate::file_picker`]) browses the filesystem by path. The app's own folder
//! needs no permission at all — that is why Save always works — but every other folder, the
//! `Documents` a user keeps their `.docx` in, `Download`, `Pictures`, is *shared* storage, and
//! Android 10+ refuses to list it unless the app holds `MANAGE_EXTERNAL_STORAGE` ("All files
//! access").
//!
//! That permission is declared in the manifest (`Cargo.toml`), but it is **not a runtime
//! permission**: there is no `requestPermissions` dialog for it, and an app that only declares
//! it silently shows a padlocked tablet. From Android 11 (API 30) on, the app itself has to send
//! the user to
//! `Settings ▸ Apps ▸ Special access ▸ All files access ▸ WordCraft` — an
//! `ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION` intent carrying the package as its data URI.
//! This module sends exactly that intent, and answers "is it granted?" so the picker can name
//! the problem instead of just drawing a lock.
//!
//! Before Android 11 the same access is the ordinary `READ_EXTERNAL_STORAGE` runtime
//! permission, so on API 26–29 the module asks for that (plus `WRITE_EXTERNAL_STORAGE`, which
//! saving needs there too) with `Activity.requestPermissions`. `android-activity` 0.6 never
//! forwards `onRequestPermissionsResult`, and nothing here needs it: the app keeps drawing its
//! own frame, and [`granted`] notices the change on a later frame.
//!
//! # Why the answer is cached
//!
//! [`granted`] is asked once per frame while the picker is open, and on API 30+ answering means
//! a call into the system. A "yes" is kept for the life of the process; a "no" only for a
//! second, so the settings screen the user was just sent to is noticed as soon as they are back.
//!
//! # What this module deliberately does not do
//!
//! Nothing here reads or writes a document. It only says whether the filesystem will let the
//! picker list a folder, and opens the one system screen that changes the answer.

/// The application id. Written a second time because the settings intent needs
/// `package:<id>` and `cargo-apk` reads the id from the manifest; the test below keeps the two
/// in agreement, because a mismatch opens *another app's* settings screen and looks like the
/// button does nothing.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub(crate) const PACKAGE: &str = "ai.storyteller.wordcraft";

/// Whether the app may read shared storage (`Documents`, `Download`, …) by path.
///
/// Outside Android nothing here is shared storage, so the answer is "no": the host build only
/// exercises the platform-independent parts of the shell.
#[cfg(not(target_os = "android"))]
pub fn granted() -> bool {
    false
}

/// Ask the system for shared-storage access, and report whether a screen was shown.
///
/// The answer arrives asynchronously (the user may take minutes in Settings), so the caller
/// should simply keep drawing: [`granted`] reports the new state once it is back.
#[cfg(not(target_os = "android"))]
pub fn request() -> bool {
    false
}

#[cfg(target_os = "android")]
pub fn granted() -> bool {
    let now = std::time::Instant::now();
    if let Ok(cache) = android::CACHE.lock()
        && let Some((at, answer)) = *cache
        && (answer || now.duration_since(at) < android::CACHE_NO)
    {
        return answer;
    }
    let answer = android::is_granted(crate::android_sdk_version()).unwrap_or(false);
    if let Ok(mut cache) = android::CACHE.lock() {
        *cache = Some((now, answer));
    }
    answer
}

#[cfg(target_os = "android")]
pub fn request() -> bool {
    // The system screen is about to change the answer; let the next frame re-read it rather than
    // keep serving the "no" that asked for the screen in the first place.
    if let Ok(mut cache) = android::CACHE.lock() {
        *cache = None;
    }
    let shown = android::request_access(crate::android_sdk_version()).is_some();
    log::info!("storage access: asked the system for shared storage (screen shown: {shown})");
    shown
}

#[cfg(target_os = "android")]
mod android {
    use std::ffi::CStr;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use jni::objects::{JObject, JObjectArray, JString, JValue};
    use jni::signature::RuntimeMethodSignature;
    use jni::strings::JNIStr;
    use jni::Env;

    use super::PACKAGE;

    /// The permission that stands in for "All files access" before Android 11.
    const READ: &str = "android.permission.READ_EXTERNAL_STORAGE";
    /// Only meaningful up to Android 9; saving into shared storage needs it there.
    const WRITE: &str = "android.permission.WRITE_EXTERNAL_STORAGE";

    /// `Activity.requestPermissions` wants an `int` request code. Only its stability within the
    /// process matters, since no result is delivered back to us; "WC" keeps it recognisable in
    /// a system log.
    const REQUEST_CODE: i32 = 0x5743;

    /// How long a "not granted" answer is trusted. A "granted" one is trusted forever: leaving
    /// the app is what would revoke it, and that stops these frames anyway.
    pub(super) const CACHE_NO: Duration = Duration::from_secs(1);

    pub(super) static CACHE: Mutex<Option<(Instant, bool)>> = Mutex::new(None);

    /// The two actions that lead to the toggle. The strings are the platform's own constants
    /// (`Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION` and
    /// `Settings.ACTION_APPLICATION_DETAILS_SETTINGS`); passing them as values keeps this code
    /// out of the `Settings` class and its field signatures.
    const ACTION_ALL_FILES: &str = "android.settings.MANAGE_APP_ALL_FILES_ACCESS_PERMISSION";
    const ACTION_APP_DETAILS: &str = "android.settings.APPLICATION_DETAILS_SETTINGS";

    /// A literal class or method name, as the JNI functions want it: NUL-terminated MUTF-8.
    fn name(n: &'static CStr) -> &'static JNIStr {
        JNIStr::from_cstr(n).expect("a literal name")
    }

    /// Run `f` on this window's Activity, attached to its JVM.
    ///
    /// Every failure — no Activity yet, a JNI error, a Java exception — is logged and turned
    /// into `None`: a permission question that cannot be answered must not take the frame down,
    /// it only means "not granted".
    fn with_env<R>(f: impl FnOnce(&mut Env, &JObject) -> jni::errors::Result<R>) -> Option<R> {
        let (vm_ptr, activity_ptr) = crate::with_android_app(|app| (app.vm_as_ptr(), app.activity_as_ptr()))?;
        // SAFETY: `vm_as_ptr` is the JavaVM this process is already attached to, and
        // `JavaVM::from_raw` is a singleton behind a non-null assert.
        let vm = unsafe { jni::JavaVM::from_raw(vm_ptr.cast()) };
        match vm.attach_current_thread(|env| {
            // SAFETY: this is this Activity's own reference, valid for as long as its `AndroidApp`
            // is. `JObject` does not own what it points at — it has no `Drop` — which is exactly
            // why android-activity's own JNI example wraps the pointer this way and warns only
            // against `Global`/`Auto`, which would delete a reference the platform still holds.
            let activity = unsafe { JObject::from_raw(env, activity_ptr as jni::sys::jobject) };
            f(env, &activity)
        }) {
            Ok(value) => Some(value),
            Err(e) => {
                log::warn!("storage access: could not run the JNI call: {e}");
                None
            }
        }
    }

    /// Ask the platform whether shared storage is readable by path.
    pub(super) fn is_granted(sdk: i32) -> Option<bool> {
        with_env(|env, activity| {
            if sdk >= 30 {
                // The API 30 way; `Environment.isExternalStorageManager` does not exist below 30,
                // which is why the whole call is behind this branch.
                let signature = RuntimeMethodSignature::from_str("()Z")?;
                let value = env.call_static_method(
                    name(c"android/os/Environment"),
                    name(c"isExternalStorageManager"),
                    signature.method_signature(),
                    &[],
                )?;
                value.z()
            } else {
                let signature = RuntimeMethodSignature::from_str("(Ljava/lang/String;)I")?;
                let permission = env.new_string(READ)?;
                let value = env.call_method(
                    activity,
                    name(c"checkSelfPermission"),
                    signature.method_signature(),
                    &[JValue::Object(&permission)],
                )?;
                // `PackageManager.PERMISSION_GRANTED` is 0.
                Ok(value.i()? == 0)
            }
        })
    }

    /// Show whichever system screen hands this access out on this Android version.
    pub(super) fn request_access(sdk: i32) -> Option<()> {
        with_env(|env, activity| {
            if sdk >= 30 {
                open_all_files_settings(env, activity)
            } else {
                request_runtime_storage(env, activity, sdk)
            }
        })
    }

    /// `Settings ▸ Apps ▸ Special access ▸ All files access ▸ WordCraft`.
    ///
    /// A few OEM Settings builds do not handle the app-specific action; on those the app's own
    /// details page is opened instead, where the same toggle is one tap deeper. A failed
    /// `startActivity` leaves a Java exception pending, so it is described and cleared before
    /// the second attempt — otherwise every later JNI call would fail too.
    fn open_all_files_settings(env: &mut Env, activity: &JObject) -> jni::errors::Result<()> {
        if let Err(e) = launch(env, activity, ACTION_ALL_FILES) {
            if env.exception_check() {
                env.exception_describe();
                env.exception_clear();
            }
            log::warn!("storage access: the All-files-access screen failed ({e}); opening app settings");
            launch(env, activity, ACTION_APP_DETAILS)?;
        }
        Ok(())
    }

    /// `Intent(action).setData(Uri.parse("package:<id>")).startActivity(activity)`.
    fn launch(env: &mut Env, activity: &JObject, action: &str) -> jni::errors::Result<()> {
        let signature = RuntimeMethodSignature::from_str;
        let ctor = signature("(Ljava/lang/String;)V")?;
        let action = env.new_string(action)?;
        let intent =
            env.new_object(name(c"android/content/Intent"), ctor.method_signature(), &[JValue::Object(&action)])?;

        let parse = signature("(Ljava/lang/String;)Landroid/net/Uri;")?;
        let data = env.new_string(format!("package:{PACKAGE}"))?;
        let uri = env
            .call_static_method(name(c"android/net/Uri"), name(c"parse"), parse.method_signature(), &[JValue::Object(&data)])?
            .l()?;

        let set_data = signature("(Landroid/net/Uri;)Landroid/content/Intent;")?;
        env.call_method(&intent, name(c"setData"), set_data.method_signature(), &[JValue::Object(&uri)])?;

        let start = signature("(Landroid/content/Intent;)V")?;
        env.call_method(activity, name(c"startActivity"), start.method_signature(), &[JValue::Object(&intent)])?;
        Ok(())
    }

    /// The pre-Android-11 equivalent: the ordinary runtime-permission dialog.
    fn request_runtime_storage(env: &mut Env, activity: &JObject, sdk: i32) -> jni::errors::Result<()> {
        let read = env.new_string(READ)?;
        // Read and write live in one permission group, so one dialog covers both; write is only
        // meaningful (and only declared) up to Android 9.
        let with_write = sdk < 29;
        let permissions = JObjectArray::<JString>::new(env, usize::from(with_write) + 1, &read)?;
        if with_write {
            let write = env.new_string(WRITE)?;
            permissions.set_element(env, 1, &write)?;
        }
        let signature = RuntimeMethodSignature::from_str("([Ljava/lang/String;I)V")?;
        let permissions: &JObject = permissions.as_ref();
        env.call_method(
            activity,
            name(c"requestPermissions"),
            signature.method_signature(),
            &[JValue::Object(permissions), JValue::Int(REQUEST_CODE)],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    /// The package id exists in two places — `Cargo.toml` (what the APK is installed as) and
    /// [`super::PACKAGE`] (what the settings intent carries) — and Android opens *another
    /// app's* settings screen if they drift apart, which looks exactly like a dead button.
    #[test]
    fn package_name_matches_the_manifest() {
        let manifest = include_str!("../Cargo.toml");
        let expected = format!("package = \"{}\"", super::PACKAGE);
        assert!(manifest.contains(&expected), "permissions.rs PACKAGE no longer matches Cargo.toml ({expected})");
    }
}
