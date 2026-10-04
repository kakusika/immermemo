plugins {
    id("com.android.application")
    kotlin("android")
}

android {
    namespace = "dev.immermemo.app"
    compileSdk = 34

    defaultConfig {
        applicationId = "dev.immermemo.app"
        minSdk = 26
        targetSdk = 34
        versionCode = 1
        versionName = "0.1.0"
    }

    // The jniLibs directory is populated by `cargo ndk` before this build
    // runs (see ../../../justfile's `apk` recipe), not by a Gradle task --
    // keeps the Rust cross-compile step independently runnable/debuggable.
    sourceSets {
        getByName("main") {
            jniLibs.srcDirs("src/main/jniLibs")
        }
    }

    packaging {
        jniLibs {
            // The whole point of this migration: cargo-apk writes
            // android:extractNativeLibs="false" to the manifest but still
            // Deflates the .so (see immermemo/Cargo.toml's old TODO), which
            // Android 11+ rejects. AGP has stored libs uncompressed and
            // page-aligned here by default since 3.6.0.
            useLegacyPackaging = false
        }
    }

    signingConfigs {
        getByName("debug") {
            // Same local debug keystore cargo-apk used (see nix/dev.nix's
            // shellHook) -- sideloading onto one's own phone, not for
            // distribution.
            storeFile =
                file(
                    System.getenv("CARGO_APK_RELEASE_KEYSTORE")
                        ?: "${System.getProperty("user.home")}/.android/debug.keystore",
                )
            storePassword = System.getenv("CARGO_APK_RELEASE_KEYSTORE_PASSWORD") ?: "android"
            keyAlias = "androiddebugkey"
            keyPassword = System.getenv("CARGO_APK_RELEASE_KEYSTORE_PASSWORD") ?: "android"
        }
    }

    buildTypes {
        release {
            signingConfig = signingConfigs.getByName("debug")
            isMinifyEnabled = false
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions {
        jvmTarget = "17"
    }
}
