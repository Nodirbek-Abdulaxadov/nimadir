//! The Linux desktop's light or dark preference, which winit does not report
//! there — on X11 or Wayland its `theme()` is always `None`, so the window was
//! light on a dark desktop. It lives in the XDG desktop portal, as
//! `org.freedesktop.appearance` `color-scheme`, on the session bus.

use eframe::egui;
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedValue, Value};

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const SETTINGS: &str = "org.freedesktop.portal.Settings";
const APPEARANCE: &str = "org.freedesktop.appearance";
const SCHEME: &str = "color-scheme";

/// Follow the desktop's preference: read it, then set the theme again each
/// time it changes. On a thread of its own, so the window never waits on the
/// bus — a portal that has to be started first can take seconds. Without a
/// session bus or a portal nothing happens, and the window stays light.
pub fn follow_color_scheme(ctx: egui::Context) {
    let spawned = std::thread::Builder::new()
        .name("color-scheme".into())
        .spawn(move || {
            if let Err(e) = watch(&ctx) {
                crate::diag::line(&format!("no desktop color scheme: {e}"));
            }
        });
    if let Err(e) = spawned {
        crate::diag::line(&format!("no desktop color scheme: {e}"));
    }
}

fn watch(ctx: &egui::Context) -> zbus::Result<()> {
    let bus = Connection::session()?;
    let settings = Proxy::new(&bus, PORTAL, PATH, SETTINGS)?;
    // Subscribed before reading, so a change in between is not missed.
    let changes = settings.receive_signal("SettingChanged")?;
    apply(ctx, read(&settings));
    for signal in changes {
        let Ok((namespace, key, value)) =
            signal.body().deserialize::<(String, String, OwnedValue)>()
        else {
            continue;
        };
        if namespace == APPEARANCE && key == SCHEME {
            apply(ctx, scheme(&value));
        }
    }
    Ok(())
}

/// `ReadOne` (portal version 2) answers with the value; the older `Read`
/// wraps it in one more variant.
fn read(settings: &Proxy) -> Option<egui::Theme> {
    if let Ok(value) = settings.call::<_, _, OwnedValue>("ReadOne", &(APPEARANCE, SCHEME)) {
        return scheme(&value);
    }
    let value: OwnedValue = settings.call("Read", &(APPEARANCE, SCHEME)).ok()?;
    match &*value {
        Value::Value(inner) => scheme(inner),
        other => scheme(other),
    }
}

/// 1 is "prefer dark", 2 "prefer light", 0 "no preference".
fn scheme(value: &Value) -> Option<egui::Theme> {
    match value {
        Value::U32(1) => Some(egui::Theme::Dark),
        Value::U32(2) => Some(egui::Theme::Light),
        _ => None,
    }
}

/// The fallback is what egui draws when the system says nothing, which on
/// Linux it never does; no preference means the shell's own default, light.
fn apply(ctx: &egui::Context, theme: Option<egui::Theme>) {
    let theme = theme.unwrap_or(egui::Theme::Light);
    if ctx.options(|o| o.fallback_theme) != theme {
        ctx.options_mut(|o| o.fallback_theme = theme);
        ctx.request_repaint();
    }
}
