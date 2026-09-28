//! Android glue: opening links with an intent. Desktop helpers like
//! `xdg-open` don't exist in an app; Android hands a URL to the app the user
//! picked for it (browser, mail) through an `ACTION_VIEW` intent.

use jni::JavaVM;
use jni::objects::{JObject, JValue};

/// Opens `url` (web or `mailto:`) in the matching app.
pub fn open_url(url: &str) -> Result<(), String> {
    let context = ndk_context::android_context();
    // SAFETY: android-activity sets up ndk-context with the process's Java VM
    // and the activity, which live as long as the app.
    let vm = unsafe { JavaVM::from_raw(context.vm().cast()) }.map_err(|e| e.to_string())?;
    let activity = unsafe { JObject::from_raw(context.context().cast()) };
    let mut env = vm.attach_current_thread().map_err(|e| e.to_string())?;
    let result = (|| {
        let text = env.new_string(url)?;
        let uri = env
            .call_static_method(
                "android/net/Uri",
                "parse",
                "(Ljava/lang/String;)Landroid/net/Uri;",
                &[JValue::Object(&text)],
            )?
            .l()?;
        let action = env.new_string("android.intent.action.VIEW")?;
        let intent = env.new_object(
            "android/content/Intent",
            "(Ljava/lang/String;Landroid/net/Uri;)V",
            &[JValue::Object(&action), JValue::Object(&uri)],
        )?;
        env.call_method(
            &activity,
            "startActivity",
            "(Landroid/content/Intent;)V",
            &[JValue::Object(&intent)],
        )?;
        Ok::<_, jni::errors::Error>(())
    })();
    if result.is_err() {
        // E.g. no app handles the URL: clear the Java exception so later
        // calls work.
        let _ = env.exception_clear();
    }
    result.map_err(|e| e.to_string())
}
