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
        // Miroir officiel Google de Maven Central : évite les limitations HTTP 429
        // de repo.maven.apache.org lors du téléchargement de Robolectric en CI.
        maven { url = uri("https://maven-central.storage-download.googleapis.com/maven2") }
        mavenCentral()
    }
}

rootProject.name = "VoiceToText"
include(":app")
