# ADR-6: The web on Android

**Status:** Accepted (2026-10-21). The plan is `docs/ROADMAP.md`, S10-3 and
S10-4.

## Context

An address that is a web page opens in the platform's webview on the
desktop (module 1, `webview.rs`, through wry). On Android it said "a phone
cannot have one yet" (S3-8): wry's Android support assumes an app built
around tao and its own Kotlin activity, and this app was a `NativeActivity`
with no Java at all — nothing that could make a `WebView`, which is a Java
view and lives on the UI thread.

Sprint 9 moved the app to GameActivity (S9-2), a Jetpack class: the app has
Java code now, and so a place to put a little more.

## Decision

**The platform's `android.webkit.WebView`, through one small Java class**,
`dev.nimadir.shell.WebPane`, driven from Rust over JNI (`android_web.rs`):

- `show(activity, url, x, y, w, h)` puts a WebView over the page area —
  physical pixels from the window's corner, the same rectangle the desktop
  gives wry — making it on first use and loading the address when it
  changes. `hide` removes and destroys it. Each runs on the UI thread; the
  native side never waits on it.
- **The same rules as the desktop.** The page area only: the chrome and the
  tabs stay the shell's, drawn above it. Leaving the page, or showing
  another tab, destroys the WebView (a hidden one would keep running the
  page's scripts). JavaScript runs, as in any browser; nothing is bridged
  to it — no `addJavascriptInterface`, so a page reaches nothing of the
  shell's.
- **Back goes back within the page first**, as in a browser on a phone,
  then through the shell's history. The WebView tells the native side
  whether it can (`canGoBack`, kept as the page's history changes).
- The class is loaded through the activity's class loader: a thread the
  native code attached sees only the system's classes.

## Consequences

- S3-8's "desktop-only for beta" ends; a web page is the same destination
  on both, and invariant 8 holds for module 1 too.
- The app carries Java of its own: `WebPane` — kept to drawing and
  removing one view.
