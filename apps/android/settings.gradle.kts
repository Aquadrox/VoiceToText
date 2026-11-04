// Application Android de VoiceToText : clavier vocal (InputMethodService)
// et réglages, en Kotlin ; le moteur (Whisper, pipeline, LLM) est la
// bibliothèque Rust `crates/vtt-android`, compilée par build-native.sh.
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

rootProject.name = "VoiceToText"
include(":app")
