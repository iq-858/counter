// Counter — Cranpose Android application.
//
// The Cranpose Gradle plugin owns the native Cargo build, ABIs, JNI packaging
// and the framework's Java/manifest contributions. Only Counter-specific
// settings live here.
plugins {
    id("com.android.application")
    id("dev.cranpose.android")
}

cranpose {
    // The Cargo package in this repository (Cargo.toml, [package] name).
    cargoPackage.set("counter")
    // Cargo names the cdylib after the package, dashes -> underscores.
    libraryName.set("counter")
    label.set("Counter")

    // Counter's own features: `android` forwards cranpose/android +
    // cranpose/renderer-wgpu. The plugin default ("renderer-wgpu") belongs to
    // Cranpose's demo crates and is not a feature of this package, so replace it.
    features.set(listOf("android"))
    // Counter's `default` feature pulls the desktop renderer; the Android build
    // must not use it.
    defaultFeatures.set(false)
    // The Gradle project lives at <repo>/android, so <repo> is two levels up
    // from <repo>/android/app (the plugin's default ../../../.. assumes the
    // demo's apps/<name>/android layout).
    workspaceRoot.set("../..")

    // HLS playback needs the INTERNET permission the plugin's `network`
    // service declares.
    services.addAll("network")

    // arm64 covers modern devices and the x86_64 image covers the emulator.
    releaseAbis.set(listOf("arm64-v8a", "x86_64"))
    releaseProfile.set("release")
}

android {
    namespace = "com.example.counter"
    compileSdk = 36

    signingConfigs {
        create("release") {
            // Values come from the CI workflow, which generates a keystore.
            // Fallbacks make a plain local `assembleRelease` work when a
            // keystore is present in android/keystore/.
            storeFile = file(
                System.getenv("COUNTER_KEYSTORE_FILE")
                    ?: "../keystore/counter-release.jks"
            )
            storePassword = System.getenv("COUNTER_STORE_PASSWORD") ?: "counter123"
            keyAlias = System.getenv("COUNTER_KEY_ALIAS") ?: "counter"
            keyPassword = System.getenv("COUNTER_KEY_PASSWORD") ?: "counter123"
        }
    }

    defaultConfig {
        applicationId = "com.example.counter"
        minSdk = 24
        targetSdk = 36
        versionCode = 1
        versionName = "1.0"
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            // A real release signing config — not the auto-generated debug key.
            signingConfig = signingConfigs.getByName("release")
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro"
            )
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}
