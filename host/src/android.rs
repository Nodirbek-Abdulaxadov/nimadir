//! Android glue the window needs and eframe does not provide: the safe area
//! (where the system's bars are not), the system's locale and theme, and
//! leaving for the background on Back.

use jni::objects::{JObject, JValue};
use jni::{JNIEnv, JavaVM};
use winit::platform::android::activity::ndk::configuration::UiModeNight;
use winit::platform::android::activity::AndroidApp;

/// What the system covers on each side of the window, in physical pixels.
#[derive(Clone, Copy, Default, PartialEq)]
pub struct Insets {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    /// Whether the on-screen keyboard is up; `bottom` is then its top edge.
    pub keyboard: bool,
}

/// The window's system-bar insets, measured once per window size.
///
/// Since API 35 an app is drawn edge to edge: the status bar, the navigation
/// bar and any camera cutout sit *over* the window, and a tap on them goes to
/// the system. Nothing in winit or eframe reports them, so they are read from
/// the activity's `WindowInsets`.
pub struct SafeArea {
    app: AndroidApp,
    insets: Option<Insets>,
    measured_for: (f32, f32),
}

impl SafeArea {
    pub fn new(app: AndroidApp) -> Self {
        SafeArea {
            app,
            insets: None,
            measured_for: (0.0, 0.0),
        }
    }

    /// The insets for a window of this size. Re-measured when the size changes
    /// — rotation, split screen — and on every frame until the first
    /// measurement succeeds, since a window not yet attached has none to give.
    /// Also on every frame while a field has focus or the keyboard is up: the
    /// keyboard covers the window without resizing it, so nothing else would
    /// say it came or went.
    pub fn insets(&mut self, size: (f32, f32), typing: bool) -> Insets {
        let keyboard = self.insets.is_some_and(|i| i.keyboard);
        if self.insets.is_none() || self.measured_for != size || typing || keyboard {
            self.insets = measure(&self.app);
            self.measured_for = size;
        }
        self.insets.unwrap_or_default()
    }
}

fn measure(app: &AndroidApp) -> Option<Insets> {
    // SAFETY: both pointers come from android-activity and stay valid for the
    // life of the activity, which outlives the window that is asking.
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) }.ok()?;
    let mut env = vm.attach_current_thread().ok()?;
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };

    // A local frame, so the references made here are freed on return: this
    // thread never goes back to Java to have them freed for it.
    let measured = env.with_local_frame(16, |env| read_insets(env, &activity));
    if measured.is_err() {
        // A pending Java exception fails every later JNI call on this thread.
        let _ = env.exception_clear();
    }
    measured.ok().flatten()
}

fn read_insets(env: &mut JNIEnv, activity: &JObject) -> jni::errors::Result<Option<Insets>> {
    let window = env
        .call_method(activity, "getWindow", "()Landroid/view/Window;", &[])?
        .l()?;
    let decor = env
        .call_method(&window, "getDecorView", "()Landroid/view/View;", &[])?
        .l()?;
    let insets = env
        .call_method(
            &decor,
            "getRootWindowInsets",
            "()Landroid/view/WindowInsets;",
            &[],
        )?
        .l()?;
    if insets.is_null() {
        return Ok(None);
    }

    let sdk = env
        .get_static_field("android/os/Build$VERSION", "SDK_INT", "I")?
        .i()?;
    if sdk >= 30 {
        let bars = env
            .call_static_method("android/view/WindowInsets$Type", "systemBars", "()I", &[])?
            .i()?;
        let cutout = env
            .call_static_method(
                "android/view/WindowInsets$Type",
                "displayCutout",
                "()I",
                &[],
            )?
            .i()?;
        let ime = env
            .call_static_method("android/view/WindowInsets$Type", "ime", "()I", &[])?
            .i()?;
        let sides = env
            .call_method(
                &insets,
                "getInsets",
                "(I)Landroid/graphics/Insets;",
                &[JValue::Int(bars | cutout)],
            )?
            .l()?;
        let typing = env
            .call_method(
                &insets,
                "getInsets",
                "(I)Landroid/graphics/Insets;",
                &[JValue::Int(ime)],
            )?
            .l()?;
        // Edge to edge, the keyboard slides over the window rather than
        // shrinking it: unless the page ends above it, the field being typed
        // into is under the keys.
        let keys = field(env, &typing, "bottom")?;
        Ok(Some(Insets {
            left: field(env, &sides, "left")?,
            top: field(env, &sides, "top")?,
            right: field(env, &sides, "right")?,
            bottom: field(env, &sides, "bottom")?.max(keys),
            keyboard: keys > 0.0,
        }))
    } else {
        // Deprecated at API 30, and the only way before it.
        Ok(Some(Insets {
            left: getter(env, &insets, "getSystemWindowInsetLeft")?,
            top: getter(env, &insets, "getSystemWindowInsetTop")?,
            right: getter(env, &insets, "getSystemWindowInsetRight")?,
            bottom: getter(env, &insets, "getSystemWindowInsetBottom")?,
            keyboard: false,
        }))
    }
}

/// The system's locale as a BCP 47 tag — `uz-UZ`, `ru-RU` — from
/// `java.util.Locale.getDefault()`. Nothing in the environment says it on
/// Android, so without this every app would think the user reads English.
pub fn system_locale(app: &AndroidApp) -> Option<String> {
    // SAFETY: as in `measure`.
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) }.ok()?;
    let mut env = vm.attach_current_thread().ok()?;
    let tag = env.with_local_frame(8, |env| -> jni::errors::Result<String> {
        let locale = env
            .call_static_method(
                "java/util/Locale",
                "getDefault",
                "()Ljava/util/Locale;",
                &[],
            )?
            .l()?;
        let tag = env
            .call_method(&locale, "toLanguageTag", "()Ljava/lang/String;", &[])?
            .l()?;
        Ok(env.get_string(&tag.into())?.into())
    });
    if tag.is_err() {
        let _ = env.exception_clear();
    }
    tag.ok().filter(|t| !t.is_empty())
}

/// Whether the system is in its dark theme. winit reports no theme on
/// Android, so it is the activity's configuration that says. The manifest
/// lists `uiMode` among the changes the activity handles, so a switch reaches
/// here instead of recreating the activity.
///
/// Read through Java, from the activity's resources: the NDK's copy
/// (`AndroidApp::config`) comes from the asset manager the activity started
/// with, and went on saying "not night" after a switch to dark. That copy is
/// only the fallback.
pub fn night(app: &AndroidApp) -> bool {
    // Configuration.UI_MODE_NIGHT_MASK and UI_MODE_NIGHT_YES.
    const MASK: i32 = 0x30;
    const YES: i32 = 0x20;
    let ndk = || app.config().ui_mode_night() == UiModeNight::Yes;
    // SAFETY: as in `measure`.
    let Ok(vm) = (unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) }) else {
        return ndk();
    };
    let Ok(mut env) = vm.attach_current_thread() else {
        return ndk();
    };
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
    let mode = env.with_local_frame(8, |env| -> jni::errors::Result<i32> {
        let resources = env
            .call_method(
                &activity,
                "getResources",
                "()Landroid/content/res/Resources;",
                &[],
            )?
            .l()?;
        let config = env
            .call_method(
                &resources,
                "getConfiguration",
                "()Landroid/content/res/Configuration;",
                &[],
            )?
            .l()?;
        env.get_field(&config, "uiMode", "I")?.i()
    });
    match mode {
        Ok(mode) => mode & MASK == YES,
        Err(_) => {
            let _ = env.exception_clear();
            ndk()
        }
    }
}

/// Dark icons on the system bars over a light theme, light ones over a dark
/// theme. Edge to edge (API 35) the bars are transparent over the window, and
/// the light theme's own icons — white, meant for a coloured bar — drew the
/// clock white on white. Done on the activity's thread, which owns its views.
pub fn set_bar_icons(app: &AndroidApp, dark_icons: bool) {
    let target = app.clone();
    app.run_on_java_main_thread(Box::new(move || {
        // SAFETY: as in `measure`.
        let Ok(vm) = (unsafe { JavaVM::from_raw(target.vm_as_ptr().cast()) }) else {
            return;
        };
        let Ok(mut env) = vm.attach_current_thread() else {
            return;
        };
        let activity = unsafe { JObject::from_raw(target.activity_as_ptr().cast()) };
        if env
            .with_local_frame(8, |env| bar_icons(env, &activity, dark_icons))
            .is_err()
        {
            let _ = env.exception_clear();
        }
    }));
}

fn bar_icons(env: &mut JNIEnv, activity: &JObject, dark: bool) -> jni::errors::Result<()> {
    let window = env
        .call_method(activity, "getWindow", "()Landroid/view/Window;", &[])?
        .l()?;
    let sdk = env
        .get_static_field("android/os/Build$VERSION", "SDK_INT", "I")?
        .i()?;
    if sdk >= 30 {
        // APPEARANCE_LIGHT_STATUS_BARS | APPEARANCE_LIGHT_NAVIGATION_BARS:
        // "light bars", which is to say dark icons.
        const MASK: i32 = 0x8 | 0x10;
        let controller = env
            .call_method(
                &window,
                "getInsetsController",
                "()Landroid/view/WindowInsetsController;",
                &[],
            )?
            .l()?;
        if !controller.is_null() {
            let on = if dark { MASK } else { 0 };
            env.call_method(
                &controller,
                "setSystemBarsAppearance",
                "(II)V",
                &[JValue::Int(on), JValue::Int(MASK)],
            )?;
        }
    } else {
        // SYSTEM_UI_FLAG_LIGHT_STATUS_BAR | SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR,
        // deprecated at API 30 and the only way before it.
        const FLAGS: i32 = 0x2000 | 0x10;
        let decor = env
            .call_method(&window, "getDecorView", "()Landroid/view/View;", &[])?
            .l()?;
        let now = env
            .call_method(&decor, "getSystemUiVisibility", "()I", &[])?
            .i()?;
        let next = if dark { now | FLAGS } else { now & !FLAGS };
        env.call_method(
            &decor,
            "setSystemUiVisibility",
            "(I)V",
            &[JValue::Int(next)],
        )?;
    }
    Ok(())
}

/// Leave for the background, as Back on an app's first screen has done since
/// Android 12: the task goes behind the launcher with the activity alive, and
/// opening the app again brings it straight back.
///
/// Not `finish`, and not closing the window: either ends eframe's event loop,
/// and winit allows one event loop per process. The process outlives the
/// activity, so the next launch would land in it, fail to make another, and
/// close at once — until Android happened to kill the process.
pub fn to_background(app: &AndroidApp) -> bool {
    // SAFETY: as in `measure`.
    let Ok(vm) = (unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) }) else {
        return false;
    };
    let Ok(mut env) = vm.attach_current_thread() else {
        return false;
    };
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
    let moved = env
        .call_method(&activity, "moveTaskToBack", "(Z)Z", &[JValue::Bool(1)])
        .and_then(|v| v.z());
    if moved.is_err() {
        let _ = env.exception_clear();
    }
    moved.unwrap_or(false)
}

fn field(env: &mut JNIEnv, obj: &JObject, name: &str) -> jni::errors::Result<f32> {
    Ok(env.get_field(obj, name, "I")?.i()? as f32)
}

fn getter(env: &mut JNIEnv, obj: &JObject, method: &str) -> jni::errors::Result<f32> {
    Ok(env.call_method(obj, method, "()I", &[])?.i()? as f32)
}
