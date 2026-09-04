plugins {
    id("com.android.application")
}

android {
    namespace = "dev.peko.bad"
    compileSdk = 33

    defaultConfig {
        applicationId = "dev.peko.bad"
        minSdk = 21
        targetSdk = 33
        versionCode = 4
    }
}

dependencies {
    implementation("com.google.android.gms:play-services-ads-identifier:18.0.1")
}
