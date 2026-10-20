// No Java or Kotlin here: the activity is the platform's own NativeActivity,
// and everything it runs is libnimadir_android.so, which build.sh puts in
// src/main/jniLibs before Gradle is called.
plugins {
    id("com.android.application")
}

// Release signing comes from the environment, so no key is ever in the tree.
// Without it the release AAB is unsigned, ready for Play App Signing.
val releaseKeystore = providers.environmentVariable("NIMADIR_KEYSTORE")
val releasePassword = providers.environmentVariable("NIMADIR_KEYSTORE_PASSWORD")
val releaseAlias = providers.environmentVariable("NIMADIR_KEY_ALIAS").orElse("nimadir")
val appVersionCode = providers.gradleProperty("versionCode").map { it.toInt() }.orElse(1)
val appVersionName = providers.gradleProperty("versionName").orElse("1.0.0-beta.1")

android {
    // A placeholder, as in Cargo.toml. Once on Play it can never change.
    namespace = "dev.nimadir.shell"
    compileSdk = 36
    // The NDK build.sh used, so Gradle can strip the library it packages
    // (50 MB with symbols) — it keeps the unstripped copy for crash reports.
    // Its version is read from the NDK itself: Gradle refuses a path whose
    // version differs from the one it expects by default.
    providers.environmentVariable("ANDROID_NDK_HOME").orNull?.let { path ->
        ndkPath = path
        file("$path/source.properties").readLines()
            .firstOrNull { it.startsWith("Pkg.Revision") }
            ?.substringAfter("=")?.trim()
            ?.let { ndkVersion = it }
    }

    defaultConfig {
        applicationId = "dev.nimadir.shell"
        minSdk = 28
        // The API level Play requires is raised every year; cargo-apk could
        // not follow it past what the NDK lists, which is why this exists.
        targetSdk = 36
        versionCode = appVersionCode.get()
        versionName = appVersionName.get()
        ndk {
            abiFilters += "arm64-v8a"
        }
    }

    signingConfigs {
        if (releaseKeystore.isPresent) {
            create("release") {
                storeFile = file(releaseKeystore.get())
                storePassword = releasePassword.orNull
                keyAlias = releaseAlias.get()
                keyPassword = releasePassword.orNull
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            signingConfig = signingConfigs.findByName("release")
        }
    }
}
