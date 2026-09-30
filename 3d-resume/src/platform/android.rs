//! Android glue: opening links with an intent, and hiding the system bars.
//! Desktop helpers like `xdg-open` don't exist in an app; Android hands a URL
//! to the app the user picked for it (browser, mail) through an
//! `ACTION_VIEW` intent.

use std::sync::OnceLock;

use jni::objects::{JObject, JValue};
use jni::refs::Cast;
use jni::{JavaVM, jni_sig, jni_str};
use winit::platform::android::activity::AndroidApp;

/// The app, for running code on Android's UI thread.
static APP: OnceLock<AndroidApp> = OnceLock::new();

/// Keeps `app` for [`hide_system_bars`].
pub fn init(app: &AndroidApp) {
    let _ = APP.set(app.clone());
}

/// Hides the status and navigation bars (immersive mode), so that they don't
/// cover the scene and the buttons; a swipe from the edge shows them for a
/// moment. Android can show them again (e.g. after another app was in
/// front), so this is called whenever the app gets the focus back.
pub fn hide_system_bars() {
    let Some(app) = APP.get() else {
        return;
    };
    // The window belongs to the UI thread, not the app's (this) thread.
    app.run_on_java_main_thread(Box::new(|| {
        if let Err(error) = hide_system_bars_now() {
            log::warn!("hiding the system bars failed: {error}");
        }
    }));
}

fn hide_system_bars_now() -> Result<(), String> {
    let context = ndk_context::android_context();
    // SAFETY: as in `open_url`.
    let vm = unsafe { JavaVM::from_raw(context.vm().cast()) };
    let activity_ref = context.context().cast();
    // Java exceptions are caught and returned as errors, so none reaches the
    // UI thread's loop.
    vm.attach_current_thread(|env| -> jni::errors::Result<()> {
        // SAFETY: as in `open_url`.
        let activity = unsafe { Cast::<JObject>::from_raw(env, &activity_ref)? };
        let window = env
            .call_method(
                &*activity,
                jni_str!("getWindow"),
                jni_sig!("()Landroid/view/Window;"),
                &[],
            )?
            .l()?;
        let sdk = env
            .get_static_field(
                jni_str!("android/os/Build$VERSION"),
                jni_str!("SDK_INT"),
                jni_sig!("I"),
            )?
            .i()?;
        if sdk >= 30 {
            let controller = env
                .call_method(
                    &window,
                    jni_str!("getInsetsController"),
                    jni_sig!("()Landroid/view/WindowInsetsController;"),
                    &[],
                )?
                .l()?;
            const BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE: i32 = 2;
            env.call_method(
                &controller,
                jni_str!("setSystemBarsBehavior"),
                jni_sig!("(I)V"),
                &[JValue::Int(BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE)],
            )?;
            let bars = env
                .call_static_method(
                    jni_str!("android/view/WindowInsets$Type"),
                    jni_str!("systemBars"),
                    jni_sig!("()I"),
                    &[],
                )?
                .i()?;
            env.call_method(
                &controller,
                jni_str!("hide"),
                jni_sig!("(I)V"),
                &[JValue::Int(bars)],
            )?;
        } else {
            // Android 8 to 10: the older flags, immersive sticky, drawn behind
            // where the bars were.
            const FLAGS: i32 = 0x1000 // SYSTEM_UI_FLAG_IMMERSIVE_STICKY
                | 0x0400 // SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
                | 0x0200 // SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION
                | 0x0100 // SYSTEM_UI_FLAG_LAYOUT_STABLE
                | 0x0004 // SYSTEM_UI_FLAG_FULLSCREEN
                | 0x0002; // SYSTEM_UI_FLAG_HIDE_NAVIGATION
            let decor = env
                .call_method(
                    &window,
                    jni_str!("getDecorView"),
                    jni_sig!("()Landroid/view/View;"),
                    &[],
                )?
                .l()?;
            env.call_method(
                &decor,
                jni_str!("setSystemUiVisibility"),
                jni_sig!("(I)V"),
                &[JValue::Int(FLAGS)],
            )?;
        }
        Ok(())
    })
    .map_err(|e| e.to_string())
}

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
