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
//! saving needs there too) with `Activity.requestPermissions`. The packaged manifest
//! also opts out of scoped storage on API 29. `android-activity` 0.6 never
//! forwards `onRequestPermissionsResult`, and nothing here needs it: the app keeps drawing its
//! own frame, and [`granted`] notices the change on a later frame.
//!
//! # Why the answer is cached
//!
//! [`granted`] is asked once per frame while the picker is open, and on API 30+ answering means
//! a call into the system. Both answers are cached for one second, so grants and
//! revocations made while the app is in the background are noticed on return.
//!
//! # What this module deliberately does not do
//!
//! Nothing here reads or writes a document. It only says whether the filesystem will let the
//! picker list a folder, and opens the one system screen that changes the answer.

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
        && now.duration_since(at) < android::CACHE_NO
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

    use jni::objects::{JObject, JValue};
    use jni::signature::RuntimeMethodSignature;
    use jni::strings::JNIStr;
    use jni::Env;



    /// The permission that stands in for "All files access" before Android 11.
    const READ: &str = "android.permission.READ_EXTERNAL_STORAGE";
    /// Cache either answer briefly; settings can change while the app is backgrounded.
    pub(super) const CACHE_NO: Duration = Duration::from_secs(1);

    pub(super) static CACHE: Mutex<Option<(Instant, bool)>> = Mutex::new(None);

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

    /// Ask the Java Activity to post the system UI request to Android's main thread.
    pub(super) fn request_access(_sdk: i32) -> Option<()> {
        with_env(|env, activity| {
            let signature = RuntimeMethodSignature::from_str("()V")?;
            env.call_method(activity, name(c"requestStorageAccess"), signature.method_signature(), &[])?;
            Ok(())
        })
    }
}
