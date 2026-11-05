import java.io.File

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

// Même version que l'application PC (Cargo.toml) : 0.15.0 → code 1500.
val version: String = File(rootDir, "../../Cargo.toml").readLines()
    .first { it.startsWith("version = ") }
    .substringAfter('"').substringBefore('"')
val (major, minor, patch) = version.split(".").map { it.toInt() }

android {
    namespace = "io.github.aquadrox.voicetotext"
    compileSdk = 36

    defaultConfig {
        applicationId = "io.github.aquadrox.voicetotext"
        minSdk = 29
        targetSdk = 36
        versionCode = major * 10_000 + minor * 100 + patch
        versionName = version
        ndk { abiFilters += "arm64-v8a" }
        manifestPlaceholders["appLabel"] = "VoiceToText"
    }

    // Clé de signature des releases : fournie par la CI (secrets), jamais
    // dans le dépôt. Sans elle, l'APK de release n'est pas signé.
    val keystore = System.getenv("ANDROID_KEYSTORE_FILE")
    signingConfigs {
        if (keystore != null) {
            create("release") {
                storeFile = file(keystore)
                storePassword = System.getenv("ANDROID_KEYSTORE_PASSWORD")
                keyAlias = System.getenv("ANDROID_KEY_ALIAS")
                keyPassword = System.getenv("ANDROID_KEY_PASSWORD")
            }
        }
    }

    // Deux éditions, même identifiant et même signature (on passe de l'une à
    // l'autre en gardant ses réglages) : « clavier » s'installe partout ;
    // « bulle » ajoute le service d'accessibilité, que Play Protect bloque
    // à l'installation hors Play Store tant qu'on ne le coupe pas.
    flavorDimensions += "edition"
    productFlavors {
        create("clavier") {
            dimension = "edition"
            buildConfigField("boolean", "BUBBLE", "false")
        }
        create("bulle") {
            dimension = "edition"
            buildConfigField("boolean", "BUBBLE", "true")
        }
    }

    buildFeatures { buildConfig = true }

    buildTypes {
        release {
            isMinifyEnabled = false
            if (keystore != null) signingConfig = signingConfigs.getByName("release")
        }
        // Version de test (PR) : s'installe à côté de la version publiée.
        debug {
            applicationIdSuffix = ".test"
            versionNameSuffix = "-test"
            manifestPlaceholders["appLabel"] = "VoiceToText (test)"
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    packaging {
        jniLibs { useLegacyPackaging = false }
    }
}

kotlin {
    compilerOptions {
        jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
    }
}
