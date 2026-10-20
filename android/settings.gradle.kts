// The Gradle side of the Android app: it only packages. The code is the Rust
// library in this same directory (Cargo.toml, src/), built by cargo-ndk into
// app/src/main/jniLibs — see build.sh, which runs both.
pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "nimadir-android"

include(":app")
