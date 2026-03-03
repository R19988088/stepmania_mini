# Android Build Split

## Local (No Java)

Local only builds native Android `.so`:

```powershell
cd mini_stepmania_rust
.\scripts\build-android-native.ps1
```

Output:

- `target/android-jniLibs/arm64-v8a/libmini_stepmania_rust.so`

Required env:

- `ANDROID_NDK_HOME` or `NDK_HOME`

## GitHub Actions (APK Packaging)

APK packaging runs in CI (`.github/workflows/android.yml`) and can use Java inside CI only.

Workflow artifacts:

- `android-native-so`
- `android-apk`
