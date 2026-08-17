//! Module 1 — the old web, rendered by the platform's own webview (`wry`).
//!
//! This is the one place in nimadir where HTML, CSS and JavaScript exist, and
//! it is quarantined here on purpose. Mini-apps (module 2) never touch it: they
//! are WASM components that describe native widgets, and nothing about them
//! changes because this file exists. A web page is simply a *different kind of
//! destination*, reached through the same address bar.
//!
//! ## How it shares the window
//!
//! The webview is created as a **child** of the eframe window rather than as a
//! window of its own, so the chrome (address bar, Home) stays drawn by egui
//! above it and keeps working while a page is open. `eframe::Frame` implements
//! `HasWindowHandle`, which is exactly what `WebViewBuilder::build_as_child`
//! wants, so no part of the existing egui setup has to be dismantled to get one.
//!
//! The child is a native surface owned by the OS, stacked over the region egui
//! would otherwise have painted. egui cannot draw *into* it and it cannot draw
//! outside its bounds — which is why the shell shows one module at a time in the
//! page area, and why the chrome has to live outside that area to survive.
//!
//! ## Linux
//!
//! On Windows and macOS the webview is an OS-level child control and there is
//! nothing to drive. On Linux `wry` is WebKitGTK, which lives in **GTK's** event
//! loop, not winit's — so the host must call `init` once before creating one and
//! `pump` on every frame, or the page loads and then freezes. `build_as_child`
//! is also X11-only there; a Wayland surface is rejected up front with a message
//! that says so, rather than being handed to wry, which would panic on it.

use anyhow::{anyhow, bail, Result};
use wry::dpi::{PhysicalPosition, PhysicalSize};
use wry::raw_window_handle::HasWindowHandle;
use wry::{Rect, WebView, WebViewBuilder};

/// Pixel bounds of the page area, relative to the window's content origin.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl From<Bounds> for Rect {
    fn from(b: Bounds) -> Rect {
        Rect {
            position: PhysicalPosition::new(b.x, b.y).into(),
            size: PhysicalSize::new(b.w, b.h).into(),
        }
    }
}

/// Initialise whatever the platform needs before a webview can exist.
/// Call once, on the main thread, before the event loop starts.
#[cfg(target_os = "linux")]
pub fn init() -> Result<()> {
    gtk::init().map_err(|e| anyhow!("could not initialise GTK for the webview: {e}"))
}

#[cfg(not(target_os = "linux"))]
pub fn init() -> Result<()> {
    Ok(())
}

/// Advance the platform's own event loop, if it has one separate from winit's.
/// Called once per egui frame; cheap when there is nothing pending.
#[cfg(target_os = "linux")]
pub fn pump() {
    // Bounded: a page that generates events faster than we drain them must not
    // be able to starve the egui frame it is drawn inside.
    let mut budget = 64;
    while budget > 0 && gtk::events_pending() {
        gtk::main_iteration_do(false);
        budget -= 1;
    }
}

#[cfg(not(target_os = "linux"))]
pub fn pump() {}

/// A live webview occupying the page area.
///
/// Dropping it destroys the native surface. The shell does exactly that when
/// you navigate away — leaving a page disposes of it, the same rule mini-apps
/// already follow. A hidden-but-alive webview would keep running scripts, and
/// timers, and audio, behind a screen that says you left.
pub struct WebPane {
    view: WebView,
    /// What is loaded now, so a repainted frame does not reload the page.
    loaded: String,
    /// What we last pushed, so a still frame does not re-issue bounds.
    bounds: Bounds,
}

impl WebPane {
    /// Create the child webview inside `parent` and point it at `url`.
    pub fn new(parent: &impl HasWindowHandle, url: &str, bounds: Bounds) -> Result<Self> {
        check_parent(parent)?;

        let view = WebViewBuilder::new()
            .with_url(url)
            .with_bounds(bounds.into())
            .build_as_child(parent)
            .map_err(|e| anyhow!("{e}"))?;

        Ok(WebPane {
            view,
            loaded: url.to_string(),
            bounds,
        })
    }

    /// Reconcile the pane with what the shell wants shown this frame.
    pub fn update(&mut self, url: &str, bounds: Bounds) -> Result<()> {
        if bounds != self.bounds {
            self.view.set_bounds(bounds.into())?;
            self.bounds = bounds;
        }
        if url != self.loaded {
            self.view.load_url(url)?;
            self.loaded = url.to_string();
        }
        Ok(())
    }
}

/// Reject a parent the platform cannot host a child webview in, *before* wry
/// gets it — wry's documented behaviour there is to panic, and a shell that
/// dies because of the display server it happens to be running under is not a
/// shell anyone can use.
#[cfg(target_os = "linux")]
fn check_parent(parent: &impl HasWindowHandle) -> Result<()> {
    use wry::raw_window_handle::RawWindowHandle;

    match parent.window_handle()?.as_raw() {
        RawWindowHandle::Xlib(_) | RawWindowHandle::Xcb(_) => Ok(()),
        RawWindowHandle::Wayland(_) => bail!(
            "a child webview needs X11, and this is a Wayland session. \
             Run the shell under XWayland (WINIT_UNIX_BACKEND=x11) to open web pages."
        ),
        _ => bail!("this window is not one a webview can attach to"),
    }
}

#[cfg(not(target_os = "linux"))]
fn check_parent(_parent: &impl HasWindowHandle) -> Result<()> {
    Ok(())
}
