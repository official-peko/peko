plugins {
    id("com.android.application")
}

android {
    namespace = "com.example.northwind"
    compileSdk = 34

    defaultConfig {
        applicationId = "com.example.northwind"
        minSdk = 24
        // Google Play requires a recent target for a new release.
        targetSdk = 31
        versionCode = 14
        versionName = "1.4.0"
    }
}

dependencies {
    implementation("com.google.android.gms:play-services-ads:22.6.0")
    implementation("com.squareup.okhttp3:okhttp:4.12.0")
}
