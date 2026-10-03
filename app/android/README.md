# Android glue

Kotlin and manifest entries for the background path. `tauri android init` generates the
Android project under `src-tauri/gen/android` (git-ignored), so `cargo xtask android`
copies these files into it after init. It is safe to run again.

- `kotlin/`: copied into `gen/android/app/src/main/java/dev/habbot/reminders/` (replacing the generated `MainActivity.kt`).
- `manifest-permissions.xml` goes before `<application>`, `manifest-application.xml` before `</application>`.

Kotlin stays thin: it schedules alarms, posts notifications and calls the Rust core through
JNI (`src-tauri/src/android.rs`). Decisions are made in Rust.
