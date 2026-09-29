//! Android glue: opening links with an intent. Desktop helpers like
//! `xdg-open` don't exist in an app; Android hands a URL to the app the user
//! picked for it (browser, mail) through an `ACTION_VIEW` intent.

use jni::objects::{JObject, JValue};
use jni::refs::Cast;
use jni::{JavaVM, jni_sig, jni_str};

/// Opens `url` (web or `mailto:`) in the matching app.
pub fn open_url(url: &str) -> Result<(), String> {
    let context = ndk_context::android_context();
    // SAFETY: android-activity sets up ndk-context with the process's Java VM
    // and the activity, which live as long as the app.
    let vm = unsafe { JavaVM::from_raw(context.vm().cast()) };
    let activity_ref = context.context().cast();
    // A Java exception thrown in here (e.g. no app handles the URL) is caught
    // and returned as an error.
    vm.attach_current_thread(|env| -> jni::errors::Result<()> {
        // SAFETY: the activity is a global reference that outlives this call;
        // `Cast` borrows it without taking ownership.
        let activity = unsafe { Cast::<JObject>::from_raw(env, &activity_ref)? };
        let text = env.new_string(url)?;
        let uri = env
            .call_static_method(
                jni_str!("android/net/Uri"),
                jni_str!("parse"),
                jni_sig!("(Ljava/lang/String;)Landroid/net/Uri;"),
                &[JValue::Object(&text)],
            )?
            .l()?;
        let action = env.new_string("android.intent.action.VIEW")?;
        let intent = env.new_object(
            jni_str!("android/content/Intent"),
            jni_sig!("(Ljava/lang/String;Landroid/net/Uri;)V"),
            &[JValue::Object(&action), JValue::Object(&uri)],
        )?;
        env.call_method(
            &*activity,
            jni_str!("startActivity"),
            jni_sig!("(Landroid/content/Intent;)V"),
            &[JValue::Object(&intent)],
        )?;
        Ok(())
    })
    .map_err(|e| e.to_string())
}
