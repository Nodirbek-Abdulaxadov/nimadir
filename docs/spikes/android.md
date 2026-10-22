# Spike: the shell on Android (ROADMAP `S1-3`)

**Date:** 2026-10-20
**Verdict: GO.** The whole host — Wasmtime with Cranelift, WASI, eframe on
wgpu, ureq — builds for Android unchanged, runs every sample app, and holds
every sandbox limit on a real phone. What is broken is the shell's manners on a
phone, not its design: five blockers, all for Sprint 3, none architectural.

## What was run

| | |
|---|---|
| Device | Xiaomi POCO X7 Pro (`rodin`), Android 16 / API 36, arm64-v8a, Mali-G720 MC7 |
| Build | `cargo-apk` 0.10.0, NDK r29 (29.0.14206865), `--release`, `aarch64-linux-android` |
| Code | `android/` — the desktop window code behind an `android_main`; components bundled with `include_bytes!` |
| APK | 14.6 MB, with five components (2.3 MB of them) inside |

```bash
ANDROID_HOME=~/Android/Sdk ANDROID_NDK_ROOT=~/Android/Sdk/ndk/29.0.14206865 \
CARGO_APK_RELEASE_KEYSTORE=~/.android/debug.keystore CARGO_APK_RELEASE_KEYSTORE_PASSWORD=android \
  cargo apk build -p nimadir-android --release
adb install -r target/release/apk/nimadir.apk
adb logcat -s nimadir RustStdoutStderr    # stdout/stderr arrive under the second tag
```

The debug keystore is for the spike only; release signing is `S8-1`.

## What works

| Check | Result |
|---|---|
| The host compiles for Android | No source change beyond splitting `host` into a library; eframe without X11, Wayland and `accesskit` |
| Cold start (`am start -W`) | 259 ms |
| Renderer | wgpu on Vulkan (Mali-G720) |
| Compile + instantiate, on device, no cache | `store` 43 KB in 221 ms · `counter` 18 KB in 73 ms · `hello` 15 KB in 28 ms · `counter-cs` **2.2 MB in 590 ms** · `runaway` 15 KB in 21 ms |
| The store | Renders and lists the registry through `list-apps` |
| Touch reaches the guest | `counter` 0 → 3 by tapping |
| Soft keyboard | Appears when the search field takes focus (`mInputShown=true`); the store filters as text arrives — but see *Open* |
| Time limit | "Spin forever" stopped after ~1 s; shell alive, same process |
| Memory limit | "Eat memory" stopped at 512 MiB; shell alive |
| Output limit (`S1-2`) | "Flood the shell" stopped; shell alive |
| A component over the network | An address typed into the store → `open-app` → downloaded, compiled, ran |
| Background → resume | State intact, surface rebuilt, no panic |

## Blockers — Sprint 3 must fix

1. ~~**The chrome is under the status bar.**~~ **Fixed the same day.** The app
   is drawn edge to edge, the status bar is 130 px tall, and the Home button
   and address bar sat inside it — taps there go to the system, so Home could
   not be reached. `android::SafeArea` now reads the window's insets over JNI,
   `ShellApp::safe_rect` lays everything out inside them with a 12 pt side
   gutter, and the address bar shrinks to leave room for Open. Verified on the
   device: chrome below the clock, Home taps back to the store.
2. **Back is swallowed.** `KEYCODE_BACK` reaches winit as a key, nothing
   handles it, and the activity neither navigates nor exits. With (1), there is
   no way out of an app at all. → `S3-5`.
3. **It never stops drawing.** 54–58 % of a core while idle on the store, and
   **99 % while in the background with no window** — `gui.rs` asks for a
   repaint every frame, and eframe keeps looping ("No window – clearing all
   surfaces"). A phone in a pocket would drain. → `S3-6`, `S3-7`.
4. **No compilation cache.** `wasmtime::Cache::from_file(None)` finds no
   config directory on Android ("failed to get the default"), so every open
   compiles from scratch. Harmless at 43 KB; at 18 MB (`counter-py`) it is a
   long wait on every visit. Give the cache the app's cache directory. →
   `S3-2`.
5. ~~**`cargo-apk` is a dead end for release.**~~ **Fixed in Sprint 3:** a
   Gradle project (AGP 9.4.1) packages the `cargo-ndk`-built library, targets
   API 36 and builds an AAB — still on NativeActivity, see below. Original
   finding: **`cargo-apk` is a dead end for release.** It accepts only SDK platforms the
   NDK lists, and NDK r29 stops at API 35, so the spike targets 35 although the
   SDK has 36. It is unmaintained, NativeActivity-only, and cannot build an
   AAB, while Play raises its required target API every year. → `S3-1` moves to
   `cargo-ndk` plus a small Gradle project.

## Should fix

- **Touch targets** are about 32 dp tall; Android's guideline is 48 dp. → `S3-3`.
- **The address bar shows the internal path**
  (`/data/user/0/dev.nimadir.shell/files/mini-apps/…`). It should show the app.
- **Errors are raw**: a DNS failure printed its whole error chain, five lines,
  into the space under the status bar. Show a sentence; log the chain.
- The chrome draws an extra title line when an app is opened from the fallback
  list, which moves the page below it — harmless, but it moved a button under a
  scripted tap.

## Open — needs a person

1. **Real keyboard input.** `adb shell input text` sends *key events*, so the
   spike proves only that path. winit 0.30's Android backend reads only
   `InputEvent::KeyEvent`; IME text events (GameActivity's) are logged as
   unknown and dropped. Whether Gboard's predictions, Cyrillic, and Uzbek
   `oʻ` / `gʻ` (U+02BB) arrive under NativeActivity has to be typed by hand.
2. **HTTPS.** The phone had no network during the test — DNS failed in the
   phone's own shell too — so TLS on the device is unverified. The `INTERNET`
   permission is granted.
3. **Rotation.** Not tried. `config_changes` is set so the activity should
   survive it rather than be recreated.

## What this changes in the plan

- **`accesskit` cannot be combined with NativeActivity** — eframe stops the
  build with `compile_error!`. Accessibility on Android (`S8-3`) needs
  GameActivity, which needs a Gradle build: one more reason for `S3-1`'s
  change of tool, and a reason to make it first.
- **Android's cleartext-HTTP block does not apply to native code.** Rust's
  sockets reach any `http://` host. `S2-4`'s HTTPS-only rule is the only guard
  on Android too.
- **The JIT is not a problem.** Cranelift's generated code runs; nothing about
  Android's memory policy stopped it.
- **The performance headroom is real** — on this device. A 2.2 MB .NET
  component in 590 ms suggests `S5-7`'s budget is reachable, but this is an
  upper-mid-range phone; the budget must be measured on a slower one.

## Later finding: GameActivity would break typing

The plan was to move to GameActivity for IME text and `accesskit`. But winit
0.30 drops the IME text events GameActivity delivers (`InputEvent::TextEvent`
is logged as unknown), while NativeActivity's fallback input connection turns
typed text into key events winit *does* read. Moving now would trade a
partial keyboard for none. The app stays on NativeActivity until IME text is
handled — a winit upgrade or a patch — which is now what `S3-4` is.

## Recommendation

Keep Android in beta (D1). Order Sprint 3 as: the Gradle/GameActivity build
(it decides the input path and accessibility), then insets and Back (they make
the app usable), then idle and background drawing, then the cache directory.

## Device run of GameActivity (2026-10-22)

S9-2 and the WebView (ADR-6), on the same Android 16 phone:

- **Typing works**, on Gboard: predictions, picking a suggestion, Backspace
  inside and past a word, Cyrillic with Russian predictions, an emoji followed
  by a word being composed (UTF-16 spans), Enter from the ✓ key, a second field
  not inheriting the first one's word.
- **Setting the keyboard's text restarts it.** GameTextInput's `setState`
  ends in `InputMethodManager.restartInput`, and the native side posts it to
  the UI thread. The first patch set the text back to the composing word after
  every change: Gboard restarted on every key, so it never showed a
  prediction, and keys `adb shell input text` sent in a burst were lost in the
  gap before the reset landed (14 of 26 letters). The patch now compares each
  text with the last and clears the keyboard only when a field changes hands.
- **The ✓ key is not a key event** but `performEditorAction`, which
  android-activity reports as `TextAction` and winit drops; it is Enter now.
- **The WebView** sits over the page area, below the tab strip when there is
  one; scrolls, follows links, types (its own input connection); Back goes back
  in the page, then leaves it; leaving or showing another tab destroys it, and
  showing the tab again makes it afresh. It refused `http://` until the
  manifest allowed cleartext. With a page whose script ticks twice a second,
  the app used 5.7 % of a core; with a still page, 0.8 % — the shell itself
  draws nothing while a page is shown.
- The phone had no network again: pages came from the computer over
  `adb reverse tcp:8765 tcp:8765` to `http://localhost:8765/`.

## The tree at speed (S9-5, 2026-10-22)

A registry of 500 cards in the app's `files/config/apps.json`, the store
opened on it (displayed in 337 ms), then flung from the top to card 466 with
`adb shell input swipe` — about twelve seconds of scrolling. SurfaceFlinger's
per-second counts for the shell's surface: 119.4–120.1 frames a second on the
120 Hz panel; the longest frame 14 ms, the first of a scroll, every other one
within 8.7 ms (`dumpsys SurfaceFlinger --latency` over the last 128: median
8.3, none over 16.7). Skipping what is off screen (S9-5's first half) is
enough on this phone; a slower one is still to try.

The thin bar on the right edge of every screenshot, at 524–871 px, is not
the page's scroll bar: it is MIUI's sidebar handle (`DockAssistantView`,
`com.miui.securitycenter`), over every app, with NativeActivity too.
