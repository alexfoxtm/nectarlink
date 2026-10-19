// SPDX-License-Identifier: GPL-3.0-or-later
import java.util.Properties
import javax.inject.Inject

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
}

// The version comes from the workspace (Cargo.toml), like the PC app's.
val cargoVersion: String = rootProject.file("../Cargo.toml").readLines()
    .first { it.startsWith("version = ") }
    .substringAfter('"').substringBefore('"')
/** 1.2.3 → 1002003: every release must have a larger code than the last. */
val cargoVersionCode: Int = cargoVersion.substringBefore('-').split('.').map(String::toInt)
    .let { (major, minor, patch) -> major * 1_000_000 + minor * 1_000 + patch }

// A fork's release sets its own identity and version, independent of upstream's
// (docs/waypair/ANDROID_BUILD.md). Each defaults to upstream's.
val appId: String = providers.gradleProperty("waypair.applicationId").orNull?.also {
    require(it.matches(Regex("[a-z][a-z0-9_]*(\\.[a-z][a-z0-9_]*)+"))) { "waypair.applicationId must be a package name" }
} ?: "app.nectarlink"
val appVersion: String = providers.gradleProperty("waypair.versionName").orNull?.also {
    require(it.isNotBlank()) { "waypair.versionName must not be blank" }
} ?: cargoVersion
val appVersionCode: Int = providers.gradleProperty("waypair.versionCode").orNull?.let {
    requireNotNull(it.toIntOrNull()?.takeIf { code -> code in 1..2_100_000_000 }) {
        "waypair.versionCode must be an integer from 1 to 2100000000"
    }
} ?: cargoVersionCode

// Release builds are signed only with an explicit key, never the debug key:
// WAYPAIR_SIGNING names a properties file (storeFile, storePassword, keyAlias,
// keyPassword); without it, upstream CI's NECTARLINK_KEYSTORE (a .jks file, key
// alias nectarlink) and NECTARLINK_KEYSTORE_PASSWORD. Release packaging fails
// without a complete key (checkReleaseSigning); debug builds and tests need none.
val releaseSigning: Map<String, String?> = System.getenv("WAYPAIR_SIGNING")?.let { path ->
    val props = Properties().apply { file(path).takeIf { it.isFile }?.inputStream()?.use { load(it) } }
    listOf("storeFile", "storePassword", "keyAlias", "keyPassword").associateWith(props::getProperty)
} ?: mapOf(
    "storeFile" to System.getenv("NECTARLINK_KEYSTORE"),
    "storePassword" to System.getenv("NECTARLINK_KEYSTORE_PASSWORD"),
    "keyAlias" to "nectarlink",
    "keyPassword" to System.getenv("NECTARLINK_KEYSTORE_PASSWORD"),
)
/** Why release signing is unusable, naming inputs but never their values; null when complete. */
val releaseSigningProblem: String? = when {
    providers.gradlePropertiesPrefixedBy("android.injected.signing.").get().isNotEmpty() ->
        "android.injected.signing.* is not accepted; use WAYPAIR_SIGNING"
    System.getenv("WAYPAIR_SIGNING")?.let { !file(it).isFile } == true -> "the WAYPAIR_SIGNING file does not exist"
    releaseSigning.values.any { it.isNullOrBlank() } ->
        "missing ${releaseSigning.filterValues { it.isNullOrBlank() }.keys.joinToString()}" +
            " (set WAYPAIR_SIGNING, or NECTARLINK_KEYSTORE and NECTARLINK_KEYSTORE_PASSWORD)"
    !file(releaseSigning.getValue("storeFile")!!).isFile -> "the keystore file does not exist"
    else -> null
}

android {
    namespace = "app.nectarlink.android"
    compileSdk = 37
    ndkVersion = "30.0.16248370"

    defaultConfig {
        applicationId = appId
        minSdk = 26
        targetSdk = 37
        versionCode = appVersionCode
        versionName = appVersion
    }

    signingConfigs {
        if (releaseSigningProblem == null) {
            create("release") {
                storeFile = file(releaseSigning.getValue("storeFile")!!)
                storePassword = releaseSigning.getValue("storePassword")
                keyAlias = releaseSigning.getValue("keyAlias")
                keyPassword = releaseSigning.getValue("keyPassword")
            }
        }
    }

    buildTypes {
        debug {
            // Its own package, so a debug build never replaces an installed app:
            // a release of this one, or legacy Waypair's com.waypair(.debug).
            applicationIdSuffix = ".nectar.debug"
        }
        release {
            signingConfig = signingConfigs.findByName("release")
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
    }

    // Lists the languages in res/ for Android 13's per-app language (App languages in
    // the system's settings, and Settings › Appearance › Language in the app).
    androidResources {
        generateLocaleConfig = true
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

}

composeCompiler {
    stabilityConfigurationFiles.add(project.layout.projectDirectory.file("compose_stability.conf"))
}

// Only packaging is gated, so release unit tests and lint still run without a key.
val checkReleaseSigning = tasks.register("checkReleaseSigning") {
    description = "Fails unless release signing is fully configured."
    val problem = releaseSigningProblem
    doLast { if (problem != null) throw GradleException("Release signing is not configured: $problem") }
}
tasks.configureEach {
    if (name in setOf("packageRelease", "packageReleaseBundle", "signReleaseBundle")) dependsOn(checkReleaseSigning)
}

// ---- The Rust core (nectarlink-ffi) ----
//
// Built with cargo-ndk for the ABIs in `nectarlink.abis` (default: devices
// and the x86_64 emulator), then UniFFI generates the Kotlin bindings from
// the built library. Both are registered as generated sources, so AGP runs
// them when needed. Rust is always built in release: debug builds of the
// core are large and slow.

/** Builds the Rust core into `<outputDir>/<abi>/libnectarlink_ffi.so`. */
abstract class CargoNdkBuild : DefaultTask() {
    @get:Inject abstract val exec: ExecOperations
    @get:Input abstract val abis: ListProperty<String>
    @get:InputDirectory @get:PathSensitive(PathSensitivity.RELATIVE) abstract val crates: DirectoryProperty
    @get:InputFile @get:PathSensitive(PathSensitivity.NONE) abstract val lockFile: RegularFileProperty
    @get:Internal abstract val workspace: DirectoryProperty
    @get:Internal abstract val ndk: DirectoryProperty
    @get:OutputDirectory abstract val outputDir: DirectoryProperty

    @TaskAction
    fun build() {
        exec.exec {
            workingDir = workspace.get().asFile
            environment("ANDROID_NDK_HOME", ndk.get().asFile.absolutePath)
            commandLine(
                buildList {
                    addAll(listOf("cargo", "ndk", "--platform", "26", "-o", outputDir.get().asFile.path))
                    abis.get().forEach { addAll(listOf("-t", it)) }
                    addAll(listOf("build", "--profile", "android", "--locked", "-p", "nectarlink-ffi"))
                },
            )
        }
    }
}

/** Generates the Kotlin bindings from a built core library. */
abstract class UniffiBindgen : DefaultTask() {
    @get:Inject abstract val exec: ExecOperations
    @get:InputFile @get:PathSensitive(PathSensitivity.NONE) abstract val library: RegularFileProperty
    @get:Internal abstract val workspace: DirectoryProperty
    @get:OutputDirectory abstract val outputDir: DirectoryProperty

    @TaskAction
    fun generate() {
        exec.exec {
            workingDir = workspace.get().asFile
            commandLine(
                "cargo", "run", "--quiet", "--locked", "-p", "nectarlink-ffi", "--features", "bindgen",
                "--bin", "uniffi-bindgen", "--", "generate", "--library", library.get().asFile.path,
                "--language", "kotlin", "--no-format", "--out-dir", outputDir.get().asFile.path,
            )
        }
    }
}

// ---- Fonts ----
//
// The shared fonts in `assets/fonts` (with their licenses) are packaged as
// app assets under `fonts/`.

/** Copies the bundled fonts and their licenses into `<outputDir>/fonts`. */
abstract class BundleFonts : DefaultTask() {
    @get:Inject abstract val fs: FileSystemOperations
    @get:InputDirectory @get:PathSensitive(PathSensitivity.RELATIVE) abstract val fonts: DirectoryProperty
    @get:OutputDirectory abstract val outputDir: DirectoryProperty

    @TaskAction
    fun copy() {
        fs.sync {
            from(fonts) { include("*.ttf", "OFL-*.txt") }
            into(outputDir.dir("fonts"))
        }
    }
}

val workspaceDir: Directory = rootProject.layout.projectDirectory.dir("..")
val rustAbis: List<String> =
    (findProperty("nectarlink.abis") as String?)?.split(",")?.map(String::trim) ?: listOf("arm64-v8a", "x86_64")

androidComponents {
    onVariants { variant ->
        val suffix = variant.name.replaceFirstChar(Char::uppercase)
        val cargo = tasks.register<CargoNdkBuild>("cargoNdkBuild$suffix") {
            description = "Builds the Rust core for Android."
            abis.set(rustAbis)
            crates.set(workspaceDir.dir("core"))
            lockFile.set(workspaceDir.file("Cargo.lock"))
            workspace.set(workspaceDir)
            ndk.set(androidComponents.sdkComponents.ndkDirectory)
        }
        val bindgen = tasks.register<UniffiBindgen>("uniffiBindgen$suffix") {
            description = "Generates the Kotlin bindings for the Rust core."
            library.set(cargo.flatMap { it.outputDir.file("${rustAbis.first()}/libnectarlink_ffi.so") })
            workspace.set(workspaceDir)
        }
        val fonts = tasks.register<BundleFonts>("bundleFonts$suffix") {
            description = "Packages the shared fonts as assets."
            this.fonts.set(workspaceDir.dir("assets/fonts"))
        }
        variant.sources.jniLibs?.addGeneratedSourceDirectory(cargo, CargoNdkBuild::outputDir)
        variant.sources.java?.addGeneratedSourceDirectory(bindgen, UniffiBindgen::outputDir)
        variant.sources.assets?.addGeneratedSourceDirectory(fonts, BundleFonts::outputDir)
    }
}

dependencies {
    implementation(platform(libs.compose.bom))
    implementation(libs.compose.ui)
    implementation(libs.compose.material3)
    implementation(libs.compose.ui.tooling.preview)
    debugImplementation(libs.compose.ui.tooling)
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    implementation(libs.androidx.lifecycle.service)
    implementation(libs.kotlinx.coroutines.android)
    implementation(libs.camerax.camera2)
    implementation(libs.camerax.lifecycle)
    implementation(libs.camerax.view)
    implementation(libs.zxing.core)
    // Elevated: the phone's own wireless debugging (pairing and shell).
    implementation(libs.libadb.android)
    implementation(libs.conscrypt.android)
    // UniFFI's Kotlin bindings call the core through JNA.
    implementation(libs.jna) { artifact { type = "aar" } }
    testImplementation(libs.junit)
}
