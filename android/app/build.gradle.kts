// SPDX-License-Identifier: GPL-3.0-or-later
import java.io.File
import java.security.MessageDigest
import java.util.Properties
import javax.inject.Inject
import org.gradle.api.artifacts.component.ModuleComponentIdentifier
import org.gradle.api.artifacts.result.ResolvedDependencyResult

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
}

// Canonical release identity shared by Link Android and desktop.
val releaseProperties = Properties().apply {
    rootProject.file("../release-version.properties").inputStream().use { load(it) }
}
val releaseVersion: String = requireNotNull(releaseProperties.getProperty("version")) {
    "release-version.properties must define version"
}.also { require(it.matches(Regex("[0-9]+\\.[0-9]+\\.[0-9]+"))) { "release version must be semantic version" } }
val releaseBuild: Int = requireNotNull(releaseProperties.getProperty("build")?.toIntOrNull()?.takeIf { it in 1..2_100_000_000 }) {
    "release-version.properties must define a positive Android build"
}
val cargoVersion: String = rootProject.file("../Cargo.toml").readLines()
    .first { it.startsWith("version = ") }
    .substringAfter('"').substringBefore('"')
require(cargoVersion == releaseVersion) { "Cargo workspace version must match release-version.properties" }

// A fork's release sets its own identity and version, independent of upstream's
// (docs/waypair/ANDROID_BUILD.md). Each defaults to upstream's.
val appId: String = providers.gradleProperty("waypair.applicationId").orNull?.also {
    require(it.matches(Regex("[a-z][a-z0-9_]*(\\.[a-z][a-z0-9_]*)+"))) { "waypair.applicationId must be a package name" }
} ?: "app.nectarlink"
val appVersion: String = providers.gradleProperty("waypair.versionName").orNull?.also {
    require(it == releaseVersion) { "waypair.versionName must match release-version.properties" }
} ?: releaseVersion
val appVersionCode: Int = providers.gradleProperty("waypair.versionCode").orNull?.let {
    requireNotNull(it.toIntOrNull()?.takeIf { code -> code == releaseBuild }) {
        "waypair.versionCode must match release-version.properties"
    }
} ?: releaseBuild
// About links this build's exact source: for a release, its archive published
// beside the APK (docs/waypair/R2_RELEASE.md). Other builds have none, and
// About says so beside the fork's repository.
val explicitSourceUrl: String? = providers.gradleProperty("waypair.sourceUrl").orNull
val sourceIsExact: Boolean = explicitSourceUrl != null || providers.gradleProperty("waypair.versionName").isPresent
val sourceUrl: String = (
    explicitSourceUrl ?: if (sourceIsExact) {
        "https://pub-a57457f4da43415ea6335550ee9e2983.r2.dev/waypair/nectar/android/releases/" +
            "v$appVersion/Waypair-Link-$appVersion-android-source.tar.gz"
    } else {
        "https://github.com/alexfoxtm/nectarlink"
    }
).also {
    require(it.matches(Regex("""https://[A-Za-z0-9._~:/?#\[\]@!$&'()*+,;=%-]+"""))) {
        "the source URL from waypair.sourceUrl or waypair.versionName must be https and URL characters only: $it"
    }
}

/** The workspace members' manifests, which with the root manifest and Cargo.lock decide the Rust graph. */
val cargoMemberManifests: List<String> = Regex("""(?s)\bmembers\s*=\s*\[(.*?)]""")
    .find(rootProject.file("../Cargo.toml").readText())?.groupValues?.get(1).orEmpty()
    .let { list -> Regex("\"([^\"]+)\"").findAll(list).map { "${it.groupValues[1]}/Cargo.toml" }.toList() }

val rustAbis: List<String> =
    (findProperty("nectarlink.abis") as String?)?.split(",")?.map(String::trim) ?: listOf("arm64-v8a", "x86_64")

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
        buildConfigField("String", "SOURCE_URL", "\"$sourceUrl\"")
        buildConfigField("boolean", "SOURCE_URL_IS_EXACT", "$sourceIsExact")
        // Dependencies contain additional ABIs; only advertise those with our Rust core.
        ndk { abiFilters += rustAbis }
    }

    // Only the languages the app offers (res/xml/locales_config.xml) go into the APK: a
    // translation still in progress stays out, so a phone in that language never shows
    // it half done (docs/translating.md).
    val offeredLocales = Regex("android:name=\"([^\"]+)\"")
        .findAll(file("src/main/res/xml/locales_config.xml").readText())
        .map { match ->
            val parts = match.groupValues[1].split('-')
            if (parts.size == 1) parts[0] else "${parts[0]}-r${parts[1]}"
        }
        .toList()
    androidResources {
        localeFilters += offeredLocales
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

/**
 * Builds the Rust core into `<outputDir>/<abi>/libnectarlink_ffi.so`.
 *
 * cargo-ndk copies only this build's cdylibs and never removes files from
 * `-o`. Dependency cdylibs (iroh, iroh-relay) carry Cargo's metadata hash in
 * their names, so without the delete a dependency change, or a dropped ABI,
 * would leave the previous copy here for the APK to package.
 */
abstract class CargoNdkBuild : DefaultTask() {
    @get:Inject abstract val exec: ExecOperations
    @get:Inject abstract val fs: FileSystemOperations
    @get:Input abstract val abis: ListProperty<String>
    @get:InputDirectory @get:PathSensitive(PathSensitivity.RELATIVE) abstract val crates: DirectoryProperty
    @get:InputFile @get:PathSensitive(PathSensitivity.NONE) abstract val lockFile: RegularFileProperty
    @get:Internal abstract val workspace: DirectoryProperty
    @get:Internal abstract val ndk: DirectoryProperty
    @get:OutputDirectory abstract val outputDir: DirectoryProperty

    @TaskAction
    fun build() {
        fs.delete { delete(outputDir) }
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

// ---- Licenses ----
//
// Packaged as app assets under `licenses/` for About › Open-source licenses:
// the repository's own licenses, the reviewed third-party notices in
// `notices/` (THIRD-PARTY-NOTICES.txt names them all), the font licenses, and
// the list of Maven modules resolved for the app.
//
// `notices/RUST-CRATES.txt` is written by `cargo xtask notices` for
// nectarlink-ffi. `notices/RUST-CRATES.inputs` records its package, target and
// crate graph and the SHA-256 of every Cargo input (root and member manifests,
// Cargo.lock, .cargo/config.toml) and of the pinned SPDX texts it used.
// Packaging fails once any of them differs; a release also fails when it
// builds an ABI whose target is neither the notices' own nor a recorded
// `covers` target, one whose crate graph was checked to lie within them
// (docs/waypair/ANDROID_BUILD.md).

/** Copies the license texts and reviewed notices into `<outputDir>/licenses`. */
abstract class BundleLicenses : DefaultTask() {
    @get:Inject abstract val fs: FileSystemOperations
    @get:InputFile @get:PathSensitive(PathSensitivity.NONE) abstract val license: RegularFileProperty
    @get:InputDirectory @get:PathSensitive(PathSensitivity.RELATIVE) abstract val licenseTexts: DirectoryProperty
    @get:InputDirectory @get:PathSensitive(PathSensitivity.RELATIVE) abstract val notices: DirectoryProperty
    @get:InputDirectory @get:PathSensitive(PathSensitivity.RELATIVE) abstract val fonts: DirectoryProperty
    @get:InputFiles @get:PathSensitive(PathSensitivity.RELATIVE) abstract val cargoInputs: ConfigurableFileCollection
    @get:Input abstract val cargoMembers: ListProperty<String>
    @get:Input abstract val rustTargets: ListProperty<String>
    @get:Input abstract val requireEveryTarget: Property<Boolean>
    @get:Internal abstract val workspace: DirectoryProperty
    @get:OutputDirectory abstract val outputDir: DirectoryProperty

    @TaskAction
    fun copy() {
        checkRustNotices()
        fs.sync {
            from(license) { rename { "LICENSE.txt" } }
            from(licenseTexts)
            from(notices) { exclude("RUST-CRATES.inputs", "RUST-CRATES.spdx-texts.json") }
            from(fonts) { include("OFL-*.txt") }
            into(outputDir.dir("licenses"))
        }
    }

    private fun checkRustNotices() {
        val root = workspace.get().asFile
        val record = notices.file("RUST-CRATES.inputs").get().asFile.readLines()
            .map(String::trim).filter { it.isNotEmpty() && !it.startsWith("#") }
        val hashLine = Regex("([0-9a-f]{64})  (\\S+)")
        val hashes = record
            .mapNotNull { hashLine.matchEntire(it)?.destructured?.let { (hash, path) -> path to hash } }
            .toMap()
        val fields = record.filterNot { hashLine.matches(it) }
            .groupBy({ it.substringBefore(' ') }, { it.substringAfter(' ') })
        val problems = mutableListOf<String>()
        (fields.keys - setOf("command", "package", "target", "crates", "covers")).forEach {
            problems += "unknown entry \"$it\""
        }
        val pkg = fields["package"]?.singleOrNull()
        val target = fields["target"]?.singleOrNull()
        if (pkg != "nectarlink-ffi" || target == null) problems += "it must name package nectarlink-ffi and one target"
        val count = fields["crates"]?.singleOrNull()?.substringBefore(' ')
        // Another target whose crate graph was checked to lie within this file's.
        val coverLine = Regex("([a-z0-9_]+(?:-[a-z0-9_]+)+) ([0-9]+) [0-9a-f]{64}")
        val covered = fields["covers"].orEmpty().mapNotNull { line ->
            val (covers, crates) = coverLine.matchEntire(line)?.destructured ?: run {
                problems += "malformed covers entry \"$line\""
                return@mapNotNull null
            }
            if (covers == target || (crates.toIntOrNull() ?: Int.MAX_VALUE) > (count?.toIntOrNull() ?: 0)) {
                problems += "covers entry \"$line\" cannot lie within the $target notices"
            }
            covers
        }
        if (covered.size != covered.toSet().size) problems += "a target is covered twice"

        val required = listOf("Cargo.toml", "Cargo.lock", "android/app/notices/RUST-CRATES.txt") + cargoMembers.get() +
            listOf(".cargo/config.toml").filter { File(root, it).isFile }
        (required - hashes.keys).forEach { problems += "$it is not recorded" }
        hashes.forEach { (path, hash) ->
            val file = File(root, path)
            val actual = if (file.isFile) {
                MessageDigest.getInstance("SHA-256").digest(file.readBytes()).joinToString("") { "%02x".format(it) }
            } else {
                "missing"
            }
            if (actual != hash) problems += "$path changed since the notices were written"
        }

        val head = File(root, "android/app/notices/RUST-CRATES.txt").useLines { it.take(12).toList() }
        if (head.firstOrNull() != "Rust crates in $pkg for $target: third-party notices") {
            problems += "RUST-CRATES.txt is not the $pkg notices for $target"
        }
        if (head.none { it == "Rust crates ($count in the $target build)" }) problems += "its crate count is not $count"
        if (problems.isNotEmpty()) {
            throw GradleException(
                "notices/RUST-CRATES.txt does not match its inputs; regenerate it and RUST-CRATES.inputs " +
                    "(docs/waypair/ANDROID_BUILD.md):\n  " + problems.joinToString("\n  "),
            )
        }

        val coveredTargets = setOfNotNull(target) + covered
        val unlisted = rustTargets.get() - coveredTargets
        if (unlisted.isNotEmpty()) {
            val message =
                "notices/RUST-CRATES.txt covers ${coveredTargets.joinToString()}, not ${unlisted.joinToString()}"
            if (requireEveryTarget.get()) {
                throw GradleException("$message: build the release only for covered ABIs, or regenerate the notices")
            }
            logger.warn("warning: $message; this non-release build's notices may miss crates")
        }
    }
}

/** The Rust target of each Android ABI that `nectarlink.abis` may name. */
val rustTargetOfAbi = mapOf(
    "arm64-v8a" to "aarch64-linux-android",
    "armeabi-v7a" to "armv7-linux-androideabi",
    "x86_64" to "x86_64-linux-android",
    "x86" to "i686-linux-android",
)

/**
 * Writes `<outputDir>/licenses/MAVEN-COMPONENTS.txt`, every Maven module
 * resolved for the app, and fails when one's group is not in `reviewedGroups`
 * (a group ending in "." covers its subgroups): a new library needs its notice
 * in `notices/` first.
 */
abstract class MavenComponents : DefaultTask() {
    @get:Input abstract val components: ListProperty<String>
    @get:Input abstract val reviewedGroups: ListProperty<String>
    @get:OutputDirectory abstract val outputDir: DirectoryProperty

    @TaskAction
    fun write() {
        val reviewed = reviewedGroups.get()
        val unreviewed = components.get().filter { component ->
            val group = component.substringBefore(':')
            reviewed.none { if (it.endsWith(".")) group.startsWith(it) else group == it }
        }
        if (unreviewed.isNotEmpty()) {
            throw GradleException("No reviewed license notice for $unreviewed: add it to notices/ and reviewedMavenGroups")
        }
        val out = outputDir.file("licenses/MAVEN-COMPONENTS.txt").get().asFile
        out.parentFile.mkdirs()
        out.writeText(buildString {
            append("Waypair Link for Android: Maven modules\n=======================================\n\n")
            append("Every Maven module resolved for this build (${components.get().size}), including\n")
            append("platform and metadata-only modules that add no code. Their licenses are in\n")
            append("THIRD-PARTY-NOTICES.txt.\n\n")
            components.get().forEach { append("$it\n") }
        })
    }
}

/** Maven groups whose licenses are reviewed in notices/THIRD-PARTY-NOTICES.txt. */
val reviewedMavenGroups = listOf(
    "androidx.", "org.jetbrains", "org.jetbrains.kotlin", "org.jetbrains.kotlinx",
    "com.google.guava", "com.google.dagger", "com.google.errorprone", "com.google.j2objc",
    "com.google.auto.value", "com.google.code.findbugs", "com.google.zxing",
    "javax.inject", "jakarta.inject", "org.jspecify", "org.checkerframework",
    "net.java.dev.jna", "org.bouncycastle", "org.conscrypt",
    "com.github.MuntashirAkon", "com.github.MuntashirAkon.spake2-java",
)

val workspaceDir: Directory = rootProject.layout.projectDirectory.dir("..")

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
        val licenses = tasks.register<BundleLicenses>("bundleLicenses$suffix") {
            description = "Packages the license texts and third-party notices as assets."
            license.set(workspaceDir.file("LICENSE"))
            licenseTexts.set(workspaceDir.dir("LICENSES"))
            notices.set(layout.projectDirectory.dir("notices"))
            this.fonts.set(workspaceDir.dir("assets/fonts"))
            cargoMembers.set(cargoMemberManifests)
            cargoInputs.from(
                workspaceDir.files("Cargo.toml", "Cargo.lock", ".cargo/config.toml", *cargoMemberManifests.toTypedArray()),
            )
            rustTargets.set(rustAbis.map { requireNotNull(rustTargetOfAbi[it]) { "nectarlink.abis: unknown ABI $it" } })
            requireEveryTarget.set(variant.buildType == "release")
            workspace.set(workspaceDir)
        }
        val mavenComponents = tasks.register<MavenComponents>("mavenComponents$suffix") {
            description = "Lists the Maven modules in the app, failing on an unreviewed one."
            reviewedGroups.set(reviewedMavenGroups)
            components.set(
                variant.runtimeConfiguration.incoming.resolutionResult.rootComponent.map { root ->
                    val seen = mutableSetOf(root)
                    val queue = ArrayDeque(listOf(root))
                    while (queue.isNotEmpty()) {
                        queue.removeFirst().dependencies.filterIsInstance<ResolvedDependencyResult>()
                            .filterNot { it.isConstraint }
                            .forEach { if (seen.add(it.selected)) queue.add(it.selected) }
                    }
                    seen.mapNotNull { (it.id as? ModuleComponentIdentifier)?.displayName }.sorted()
                },
            )
        }
        variant.sources.jniLibs?.addGeneratedSourceDirectory(cargo, CargoNdkBuild::outputDir)
        variant.sources.java?.addGeneratedSourceDirectory(bindgen, UniffiBindgen::outputDir)
        variant.sources.assets?.addGeneratedSourceDirectory(fonts, BundleFonts::outputDir)
        variant.sources.assets?.addGeneratedSourceDirectory(licenses, BundleLicenses::outputDir)
        variant.sources.assets?.addGeneratedSourceDirectory(mavenComponents, MavenComponents::outputDir)
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
    // Local JVM tests get android.jar's org.json stubs, which throw; this is
    // the real implementation for the manifest parsing tests.
    testImplementation("org.json:json:20260814")
}
