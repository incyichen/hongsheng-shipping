plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "tw.hongsheng.handheld"
    compileSdk = 34

    defaultConfig {
        applicationId = "tw.hongsheng.handheld"
        minSdk = 26
        targetSdk = 34
        versionCode = (System.getenv("APP_VERSION_CODE") ?: "1").toInt()
        versionName = System.getenv("APP_VERSION_NAME") ?: "0.5.1"
    }

    // 固定簽章：之後的新版可以直接覆蓋安裝，不用先移除（移除會清掉手持機上的資料）
    signingConfigs {
        create("inhouse") {
            storeFile = file("hongsheng-handheld.jks")
            storePassword = "hongsheng-handheld"
            keyAlias = "handheld"
            keyPassword = "hongsheng-handheld"
        }
    }
    buildTypes {
        getByName("release") {
            isMinifyEnabled = false
            signingConfig = signingConfigs.getByName("inhouse")
        }
        getByName("debug") {
            signingConfig = signingConfigs.getByName("inhouse")
        }
    }
    buildFeatures { buildConfig = true }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }

    // 畫面（HTML/JS）直接用 handheld/web 的檔案
    sourceSets["main"].assets.srcDirs("../../web")
}

dependencies {
    implementation("androidx.appcompat:appcompat:1.7.0")
    implementation("androidx.webkit:webkit:1.11.0")
    implementation("com.sunmi:printerlibrary:1.0.18")
}
