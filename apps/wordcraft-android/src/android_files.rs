//! Poll this Activity's inbox of documents received through Android ACTION_VIEW.
//! The Java worker copies content URIs while their temporary read grants are valid.
//! Normal in-app Open continues to use the built-in file picker and original paths.
use jni::objects::JObject;
use jni::signature::RuntimeMethodSignature;
use jni::strings::JNIStr;

pub fn take_opened_document() -> Option<String> {
    crate::with_android_app(|app| {
        // SAFETY: both handles belong to this live Activity and are used only during this call.
        let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) };
        vm.attach_current_thread(|env| -> jni::errors::Result<Option<String>> {
            let activity = unsafe { JObject::from_raw(env, app.activity_as_ptr().cast()) };
            let signature = RuntimeMethodSignature::from_str("()Ljava/lang/String;")?;
            let method = JNIStr::from_cstr(c"takeOpenedDocument")
                .ok_or(jni::errors::Error::NullPtr("method name"))?;
            let value = env.call_method(&activity, method, signature.method_signature(), &[])?.l()?;
            if value.is_null() {
                Ok(None)
            } else {
                let value = jni::objects::JString::cast_local(env, value)?;
                Ok(Some(value.to_string()))
            }
        }).map_err(|e| log::warn!("Android file inbox: {e}")).ok().flatten()
    }).flatten()
}
