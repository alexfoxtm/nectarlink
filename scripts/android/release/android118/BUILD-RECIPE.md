# Android118 normal personal build

Use the exact reviewed commit/tree named by the release source offer. The owner
runs the adjacent shell recipe with an installed JDK17, SDK37, matching retained
NDK/Rust/Gradle dependencies and canonical Cargo cache. The existing Gradle
wrapper supplies all native and UniFFI generation; no old merged library may be
substituted. Offline/locked dependencies and existing caches are required.

Identity stays com.waypair, arm64-v8a, versionCode118/version0.1.4-local.25.
Metadata travels through the actual existing waypair Gradle properties. Generic
app qualification flags remain at their reviewed OFF defaults; no grant or
feature-enable flags are supplied. The current owner's existing release signer
is used through WAYPAIR_SIGNING; no signing key is distributed with source, and
reproducing a source build does not promise the owner's signature bytes.

The private release operator separately verifies complete final-source manifest,
exact source checks, generated ABI, lint/native closure, source-offer blobs,
actual APK signer/permissions/components/resources/licenses and independent
personal artifact review. Installation, immutable source/artifact upload,
protected feed changes, HTTP/OTA readback and physical-device acceptance remain
separate. No camera activation, readiness or Zoom acceptance follows from build.
