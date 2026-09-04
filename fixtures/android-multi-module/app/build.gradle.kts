plugins {
    alias(libs.plugins.android.application)
}

android {
    namespace = "dev.peko.multi"
    compileSdk = 34

    defaultConfig {
        applicationId = "dev.peko.multi"
        minSdk = 24
        targetSdk = 34
    }

    buildTypes {
        release { }
        debug { }
    }
}
