#!/usr/bin/env bash
# Build the Android app, in two steps:
#
#   1. cargo ndk  -> app/src/main/jniLibs/arm64-v8a/libnimadir_android.so
#      (the whole shell; the components it carries are compiled in)
#   2. gradlew    -> a debug APK to install, and a release AAB for Play
#
#   ./android/build.sh            build both
#   ./android/build.sh install    build, then install the APK on the device adb sees
#
# Needs ANDROID_HOME, an NDK under it, `rustup target add aarch64-linux-android`
# and `cargo install cargo-ndk`; the mini-apps must be built first
# (./build-mini-apps.sh), since the library bundles them.
set -euo pipefail
cd "$(dirname "$0")"

: "${ANDROID_HOME:?set ANDROID_HOME to the Android SDK}"
if [[ -z "${ANDROID_NDK_HOME:-}" ]]; then
  ANDROID_NDK_HOME=$(ls -d "$ANDROID_HOME"/ndk/* 2>/dev/null | sort -V | tail -1)
fi
export ANDROID_NDK_HOME
[[ -d "$ANDROID_NDK_HOME" ]] || { echo "no NDK under $ANDROID_HOME/ndk" >&2; exit 1; }

# Release Rust in both packages: Cranelift compiling guests in a debug build
# is slow enough to look broken on a phone.
cargo ndk -t arm64-v8a -P 28 -o app/src/main/jniLibs build -p nimadir-android --release

./gradlew --quiet assembleDebug bundleRelease

APK=app/build/outputs/apk/debug/app-debug.apk
AAB=app/build/outputs/bundle/release/app-release.aab
ls -la "$APK" "$AAB"

if [[ "${1:-}" == "install" ]]; then
  "$ANDROID_HOME/platform-tools/adb" install -r "$APK"
fi
