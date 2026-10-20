//! Android glue the window needs and eframe does not provide. Only the safe
//! area, so far: where the system's bars are not.

use jni::objects::{JObject, JValue};
use jni::{JNIEnv, JavaVM};
use winit::platform::android::activity::AndroidApp;

/// What the system covers on each side of the window, in physical pixels.
#[derive(Clone, Copy, Default, PartialEq)]
pub struct Insets {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
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
    pub fn insets(&mut self, size: (f32, f32)) -> Insets {
        if self.insets.is_none() || self.measured_for != size {
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
        let sides = env
            .call_method(
                &insets,
                "getInsets",
                "(I)Landroid/graphics/Insets;",
                &[JValue::Int(bars | cutout)],
            )?
            .l()?;
        Ok(Some(Insets {
            left: field(env, &sides, "left")?,
            top: field(env, &sides, "top")?,
            right: field(env, &sides, "right")?,
            bottom: field(env, &sides, "bottom")?,
        }))
    } else {
        // Deprecated at API 30, and the only way before it.
        Ok(Some(Insets {
            left: getter(env, &insets, "getSystemWindowInsetLeft")?,
            top: getter(env, &insets, "getSystemWindowInsetTop")?,
            right: getter(env, &insets, "getSystemWindowInsetRight")?,
            bottom: getter(env, &insets, "getSystemWindowInsetBottom")?,
        }))
    }
}

fn field(env: &mut JNIEnv, obj: &JObject, name: &str) -> jni::errors::Result<f32> {
    Ok(env.get_field(obj, name, "I")?.i()? as f32)
}

fn getter(env: &mut JNIEnv, obj: &JObject, method: &str) -> jni::errors::Result<f32> {
    Ok(env.call_method(obj, method, "()I", &[])?.i()? as f32)
}
