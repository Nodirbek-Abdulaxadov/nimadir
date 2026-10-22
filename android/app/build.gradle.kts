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

// The shell's version is the host crate's — what `--version`, the log and the
// update check say — so there is one place to bump it.
val shellVersion = rootProject.file("../host/Cargo.toml").readLines()
    .first { it.startsWith("version = ") }
    .substringAfter('"').substringBefore('"')

// Play refuses a versionCode that is not larger than the last one, so it is
// worked out from the version, in an order that only grows:
// MM mmm ppp sss — major, minor, patch, then the stage: alpha.N is 100+N,
// beta.N 300+N, rc.N 500+N, and the release itself 999.
fun versionCodeOf(version: String): Int {
    val core = version.substringBefore('-')
    val pre = version.substringAfter('-', "")
    val (major, minor, patch) = core.split('.').map { it.toInt() }
    val n = pre.substringAfter('.', "0").toInt()
    val stage = when (pre.substringBefore('.')) {
        "" -> 999
        "alpha" -> 100 + n
        "beta" -> 300 + n
        "rc" -> 500 + n
        else -> error("versionCode: unknown pre-release '$pre' in $version")
    }
    require(major <= 20 && minor < 100 && patch < 1000 && n < 200) {
        "versionCode: $version does not fit"
    }
    return major * 100_000_000 + minor * 1_000_000 + patch * 1_000 + stage
}

val appVersionCode = providers.gradleProperty("versionCode").map { it.toInt() }
    .orElse(versionCodeOf(shellVersion))
val appVersionName = providers.gradleProperty("versionName").orElse(shellVersion)

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

dependencies {
    // The activity the native library runs behind (S9-2): GameActivity's
    // input connection gives the soft keyboard's text, which NativeActivity
    // cannot. The version android-activity 0.6 is written against; its C
    // glue is android-activity's own, so no prefab.
    implementation("androidx.games:games-activity:4.4.0")
    implementation("androidx.appcompat:appcompat:1.7.1")
    // Those pull two Kotlin standard libraries, 1.8's and the split jdk7/jdk8
    // ones of 1.6, which hold the same classes; the BOM makes them one.
    implementation(platform("org.jetbrains.kotlin:kotlin-bom:1.8.22"))
}
