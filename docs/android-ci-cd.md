# WordCraft Android CI/CD

This repository uses separate workflows so everyday source edits remain fast and release builds are deliberate.

## Delivery stages

| Stage | Workflow | Trigger | Output |
| --- | --- | --- | --- |
| 1. Fast source gate | [Android CI - Fast](../.github/workflows/android-rust-check.yml) | Relevant code push/PR or manual run | Rust formatting result and Android ARM64 compile check |
| 2. Device-test package | [Android QA APK](../.github/workflows/android-feasibility.yml) | App entry-point/UI-layer or Android packaging changes, PR, or manual run | Temporary-signed APK + SHA-256 + build metadata (14-day artifact) |
| 3. Versioned release | [Android Release](../.github/workflows/android-release.yml) | Tag android-vMAJOR.MINOR.PATCH or manual version input | Optimized APK with persistent signature, checksum and GitHub Release |

The QA workflow builds arm64-v8a (phones) and x86_64 (emulator) native libraries, inspects the APK package contents and signature, and uploads only after verification. Its temporary signing key is deliberately discarded; Android treats each QA build as a different signer. To install a new QA build over an existing QA build, uninstall the old app first.

A release is not published unless source formatting, the core geometry/document/layout tests, native compilation, Gradle packaging, APK alignment, package ID, required ABI libraries and APK signature verification pass, and the signed APK installs and remains alive after launching on an Android 16/API 36 emulator.

## One-time release signing setup

A release must use the same persistent keystore for every version. Losing or rotating this key prevents existing users from installing updates over the current app.

1. On a trusted computer, create a private Android signing keystore and store a secure backup:
   keytool -genkeypair -v -keystore wordcraft-release.jks -alias wordcraft-release -keyalg RSA -keysize 4096 -validity 10000
2. Base64-encode the file without adding line breaks. On Linux use: base64 -w0 wordcraft-release.jks. On macOS use: base64 < wordcraft-release.jks | tr -d '\n'.
3. Open GitHub → Ravi-katta-dev/wordcraft → Settings → Secrets and variables → Actions → New repository secret. Add:
   - ANDROID_KEYSTORE_BASE64: base64 output of the keystore file
   - ANDROID_KEYSTORE_PASSWORD: keystore password
   - ANDROID_KEY_ALIAS: alias used when creating the key
   - ANDROID_KEY_PASSWORD: private-key password

Do not commit the keystore, passwords, or base64 output to Git. Never rotate the key just to fix a CI failure.

## Publishing a release

After adding the signing secrets and checking the fast workflow, create and push a tag from the exact commit to release, for example:

    git tag android-v0.5.1
    git push origin android-v0.5.1

The Android Release workflow tests, builds, signs, verifies, calculates the SHA-256 checksum and publishes WordCraft-Android-0.5.1.apk to a GitHub Release. Android versionCode is derived deterministically from MAJOR.MINOR.PATCH; each component must be less than 1000.

GitHub only exposes the manual `workflow_dispatch` button after this workflow exists on the repository's default branch (`main`). Until the workflow is merged to `main`, use the version-tag method above from a commit that contains `.github/workflows/android-release.yml`. After it is on `main`, you can also use Actions → Android Release → Run workflow, select the intended branch/commit, and enter a version such as `0.5.1`.

On the working `pr1` branch, changes under `apps/wordcraft/**`, `crates/ui-egui/**` or `android/**` automatically start the QA package build. The manual Run workflow button is only available after the workflow is present on the default branch (`main`).

## Getting the QA APK

Open Actions → Android QA APK → latest successful run → Artifacts and download wordcraft-android-qa-<commit SHA>. Extract the artifact and verify the SHA256SUMS file before installing the APK. Because the QA signer is ephemeral, uninstall an older QA build before installing a new one.

## Testing limits

CI verifies formatting, selected core unit tests on release, Android-target compilation, native ABI presence, APK structure, alignment, cryptographic signature, and a short Android 16 emulator startup smoke test. It does not prove keyboard behavior, text-editing usability, GPU compatibility on every ROM or complete end-to-end app operation. Those still need hands-on phone testing; record device model/Android version, exact steps and logs when reporting failures.
