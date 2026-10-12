plugins {
    id("com.android.application")
}

val wordcraftVersionName = providers.gradleProperty("wordcraftVersionName").orElse("0.5.0").get()
val wordcraftVersionCode = providers.gradleProperty("wordcraftVersionCode").orElse("1").get().toInt()

android {
    namespace = "ai.storyteller.wordcraft"
    compileSdk = 35

    defaultConfig {
        applicationId = "ai.storyteller.wordcraft"
        minSdk = 24
        targetSdk = 35
        versionCode = wordcraftVersionCode
        versionName = wordcraftVersionName
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

configurations.configureEach {
    exclude(group = "org.jetbrains.kotlin", module = "kotlin-stdlib-jdk7")
    exclude(group = "org.jetbrains.kotlin", module = "kotlin-stdlib-jdk8")
}

dependencies {
    implementation("androidx.games:games-activity:4.4.0")
    implementation("androidx.appcompat:appcompat:1.7.0")
    implementation("androidx.core:core:1.15.0")
}
