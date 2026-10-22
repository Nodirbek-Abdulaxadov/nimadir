//! A web page on Android (ADR-6): `dev.nimadir.shell.WebPane`, the platform's
//! WebView over the page area, driven over JNI. What `webview::WebPane` is on
//! the desktop: made on first use, told only what changed, and destroyed
//! when dropped — leaving the page, or showing another tab.

use jni::objects::{JClass, JObject, JValue};
use jni::{JNIEnv, JavaVM};
use winit::platform::android::activity::AndroidApp;

const CLASS: &str = "dev.nimadir.shell.WebPane";

/// Where the page is, in physical pixels from the window's corner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bounds {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

pub struct AndroidWeb {
    app: AndroidApp,
    url: String,
    bounds: Bounds,
}

impl AndroidWeb {
    pub fn new(app: AndroidApp) -> Self {
        AndroidWeb {
            app,
            url: String::new(),
            bounds: Bounds::default(),
        }
    }

    /// Show `url` over `bounds`, telling the WebView only what changed.
    pub fn update(&mut self, url: &str, bounds: Bounds) -> anyhow::Result<()> {
        if url == self.url && bounds == self.bounds {
            return Ok(());
        }
        let Bounds { x, y, w, h } = bounds;
        let address = url.to_string();
        call(&self.app, |env, class, activity| {
            let url = env.new_string(&address)?;
            env.call_static_method(
                class,
                "show",
                "(Landroid/app/Activity;Ljava/lang/String;IIII)V",
                &[
                    JValue::Object(activity),
                    JValue::Object(&url),
                    JValue::Int(x),
                    JValue::Int(y),
                    JValue::Int(w),
                    JValue::Int(h),
                ],
            )?;
            Ok(())
        })?;
        self.url = url.to_string();
        self.bounds = bounds;
        Ok(())
    }

    /// Whether the page has somewhere to go back to.
    pub fn can_go_back(&self) -> bool {
        call(&self.app, |env, class, _| {
            env.call_static_method(class, "canGoBack", "()Z", &[])?.z()
        })
        .unwrap_or(false)
    }

    /// Back within the page.
    pub fn go_back(&self) {
        let _ = call(&self.app, |env, class, activity| {
            env.call_static_method(
                class,
                "goBack",
                "(Landroid/app/Activity;)V",
                &[JValue::Object(activity)],
            )?;
            Ok(())
        });
    }
}

impl Drop for AndroidWeb {
    fn drop(&mut self) {
        let _ = call(&self.app, |env, class, activity| {
            env.call_static_method(
                class,
                "hide",
                "(Landroid/app/Activity;)V",
                &[JValue::Object(activity)],
            )?;
            Ok(())
        });
    }
}

/// Run `f` with the `WebPane` class and the activity. The class comes
/// through the activity's class loader: a thread the native side attached
/// sees only the system's classes.
fn call<T>(
    app: &AndroidApp,
    f: impl FnOnce(&mut JNIEnv, &JClass, &JObject) -> jni::errors::Result<T>,
) -> anyhow::Result<T> {
    // SAFETY: both pointers come from android-activity and stay valid for
    // the life of the activity, which outlives the window that is asking.
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) }?;
    let mut env = vm.attach_current_thread()?;
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
    let result = (|| {
        let loader = env
            .call_method(
                &activity,
                "getClassLoader",
                "()Ljava/lang/ClassLoader;",
                &[],
            )?
            .l()?;
        let name = env.new_string(CLASS)?;
        let class = env
            .call_method(
                &loader,
                "loadClass",
                "(Ljava/lang/String;)Ljava/lang/Class;",
                &[JValue::Object(&name)],
            )?
            .l()?;
        let class = JClass::from(class);
        f(&mut env, &class, &activity)
    })();
    if result.is_err() {
        let _ = env.exception_clear();
    }
    Ok(result?)
}
