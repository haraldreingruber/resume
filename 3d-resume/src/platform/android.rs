//! Android glue: opening links with an intent, and hiding the system bars.
//! Desktop helpers like `xdg-open` don't exist in an app; Android hands a URL
//! to the app the user picked for it (browser, mail) through an
//! `ACTION_VIEW` intent.

use std::sync::Mutex;

use jni::objects::{JObject, JValue};
use jni::refs::Cast;
use jni::{Env, JavaVM, jni_sig, jni_str};
use winit::platform::android::activity::AndroidApp;

/// The app: its Java VM and activity, and running code on Android's UI
/// thread. Replaced if `android_main` runs again.
static APP: Mutex<Option<AndroidApp>> = Mutex::new(None);

/// Keeps `app` for the functions here.
pub fn init(app: &AndroidApp) {
    *APP.lock().unwrap() = Some(app.clone());
}

fn app() -> Result<AndroidApp, String> {
    APP.lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "no Android app yet".to_owned())
}

/// Calls `f` with a JNI environment and the activity. Not the context from
/// `ndk-context`, which android-activity sets to the `Application`: it has
/// no window, and starts activities only in a new task. Java exceptions are
/// caught and returned as errors.
fn with_activity<T>(
    app: &AndroidApp,
    f: impl FnOnce(&mut Env, &JObject) -> jni::errors::Result<T>,
) -> Result<T, String> {
    // SAFETY: the process's Java VM, which lives as long as the process.
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) };
    let activity_ref: jni::sys::jobject = app.activity_as_ptr().cast();
    vm.attach_current_thread(|env| {
        // SAFETY: a global reference that stays valid while `app` exists;
        // `Cast` borrows it without taking ownership.
        let activity = unsafe { Cast::<JObject>::from_raw(env, &activity_ref)? };
        f(env, &activity)
    })
    .map_err(|e| e.to_string())
}

/// Opens `url` (web or `mailto:`) in the matching app.
pub fn open_url(url: &str) -> Result<(), String> {
    // E.g. no app handles the URL: an exception, returned as an error.
    with_activity(&app()?, |env, activity| {
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
            activity,
            jni_str!("startActivity"),
            jni_sig!("(Landroid/content/Intent;)V"),
            &[JValue::Object(&intent)],
        )?;
        Ok(())
    })
}

/// Hides the status and navigation bars (immersive mode), so that they don't
/// cover the scene and the buttons; a swipe from the edge shows them for a
/// moment. Android can show them again (e.g. after another app was in
/// front), so this is called whenever the app gets the focus back.
pub fn hide_system_bars() {
    let Ok(app) = app() else {
        return;
    };
    // The window belongs to the UI thread, not the app's (this) thread.
    app.clone().run_on_java_main_thread(Box::new(move || {
        match with_activity(&app, hide_system_bars_now) {
            Ok(sdk) => log::info!("system bars hidden (Android API level {sdk})"),
            Err(error) => log::warn!("hiding the system bars failed: {error}"),
        }
    }));
}

/// Returns the Android API level.
fn hide_system_bars_now(env: &mut Env, activity: &JObject) -> jni::errors::Result<i32> {
    let window = env
        .call_method(
            activity,
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
    Ok(sdk)
}
