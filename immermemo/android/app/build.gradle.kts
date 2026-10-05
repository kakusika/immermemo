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

dependencies {
    // Holds the system splash screen open past NativeActivity's own
    // startup (see MainActivity.kt) until Slint's first frame has
    // rendered. Pinned to 1.0.1, not the newer 1.2.0: 1.2.0's AAR
    // metadata requires compileSdk 35+, and the next stable line after
    // 1.0.1 (1.1.0) never shipped past -rc01 -- compileSdk here stays
    // at 34 since this nix dev shell only provides the android-34 SDK
    // platform.
    implementation("androidx.core:core-splashscreen:1.0.1")
}
