//! The Android side of the Rust core: JNI exports that Kotlin calls from the alarm,
//! boot and notification receivers without starting the Activity, and calls the other
//! way into the Kotlin helpers.
//!
//! The exports open their own `Core` on the database file each time: Tauri's managed
//! state doesn't exist when Android starts the process for a receiver.

use hab_core::{Core, Millis};
use jni::objects::{JClass, JObject, JString, JValue};
use jni::sys::{jboolean, jlong, jstring};
use jni::JNIEnv;

/// The user's name in the skeleton, as in `lib.rs`.
const USER: &str = "me";
const PACKAGE: &str = "dev.habbot.reminders";

fn with_core<T>(db: &str, f: impl FnOnce(&mut Core) -> hab_core::Result<T>) -> Option<T> {
    let mut core = Core::open(db, USER).ok()?;
    f(&mut core).ok()
}

/// `Native.fire(db, now)`: fires whatever is due and returns the occurrences opened
/// as a JSON array of `{id, title}`, or an empty array.
#[no_mangle]
pub extern "system" fn Java_dev_habbot_reminders_Native_fire<'l>(
    mut env: JNIEnv<'l>,
    _class: JClass<'l>,
    db: JString<'l>,
    now: jlong,
) -> jstring {
    let db: String = env.get_string(&db).map(Into::into).unwrap_or_default();
    let opened = with_core(&db, |core| core.fire_due(now)).unwrap_or_default();
    let json: Vec<_> = opened
        .iter()
        .map(|o| serde_json::json!({ "id": o.id, "title": o.title }))
        .collect();
    env.new_string(serde_json::Value::Array(json).to_string())
        .map(|s| s.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

/// `Native.complete(db, occurrenceId, now)`: completes an open occurrence.
#[no_mangle]
pub extern "system" fn Java_dev_habbot_reminders_Native_complete<'l>(
    mut env: JNIEnv<'l>,
    _class: JClass<'l>,
    db: JString<'l>,
    occurrence_id: JString<'l>,
    now: jlong,
) -> jboolean {
    let db: String = env.get_string(&db).map(Into::into).unwrap_or_default();
    let id: String = env
        .get_string(&occurrence_id)
        .map(Into::into)
        .unwrap_or_default();
    with_core(&db, |core| core.complete(&id, now)).is_some() as jboolean
}

/// `Native.nextDue(db)`: when the next unfired reminder comes due, or -1.
#[no_mangle]
pub extern "system" fn Java_dev_habbot_reminders_Native_nextDue<'l>(
    mut env: JNIEnv<'l>,
    _class: JClass<'l>,
    db: JString<'l>,
) -> jlong {
    let db: String = env.get_string(&db).map(Into::into).unwrap_or_default();
    with_core(&db, |core| Ok(core.next_due()))
        .flatten()
        .unwrap_or(-1)
}

/// Runs `f` with an attached JNI environment and the application context.
fn with_context<T>(f: impl FnOnce(&mut JNIEnv, &JObject) -> jni::errors::Result<T>) -> Option<T> {
    let ctx = ndk_context::android_context();
    // SAFETY: the pointers come from ndk-context, which wry initialises with the JavaVM
    // and a global reference to the Android context for the life of the process.
    let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) }.ok()?;
    let context = unsafe { JObject::from_raw(ctx.context().cast()) };
    let mut env = vm.attach_current_thread().ok()?;
    f(&mut env, &context).ok()
}

/// Calls a `@JvmStatic` method of one of our Kotlin classes. App classes can't be found
/// with `FindClass` from a thread Rust created, so the class comes from the context's
/// class loader.
fn call_static<'l>(
    env: &mut JNIEnv<'l>,
    context: &JObject,
    class: &str,
    method: &str,
    sig: &str,
    args: &[JValue],
) -> jni::errors::Result<jni::objects::JValueOwned<'l>> {
    let loader = env
        .call_method(context, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?
        .l()?;
    let name = env.new_string(format!("{PACKAGE}.{class}"))?;
    let class = env
        .call_method(
            &loader,
            "loadClass",
            "(Ljava/lang/String;)Ljava/lang/Class;",
            &[JValue::Object(&name)],
        )?
        .l()?;
    env.call_static_method(JClass::from(class), method, sig, args)
}

/// The app's private files directory: the same place Kotlin keeps `reminders.db`.
pub fn files_dir() -> Option<String> {
    with_context(|env, context| {
        let dir = env
            .call_method(context, "getFilesDir", "()Ljava/io/File;", &[])?
            .l()?;
        let path = env
            .call_method(&dir, "getAbsolutePath", "()Ljava/lang/String;", &[])?
            .l()?;
        let path: String = env.get_string(&JString::from(path))?.into();
        Ok(path)
    })
}

/// Registers the exact alarm for the next reminder (or cancels it when `at` is `None`).
pub fn schedule_alarm(at: Option<Millis>) {
    with_context(|env, context| {
        call_static(
            env,
            context,
            "Alarms",
            "scheduleAt",
            "(Landroid/content/Context;J)V",
            &[JValue::Object(context), JValue::Long(at.unwrap_or(-1))],
        )
        .map(|_| ())
    });
}

/// The permissions the app still needs, as names (`notifications`, `alarms`).
pub fn missing_permissions() -> Vec<String> {
    with_context(|env, context| {
        let v = call_static(
            env,
            context,
            "Permissions",
            "missing",
            "(Landroid/content/Context;)Ljava/lang/String;",
            &[JValue::Object(context)],
        )?
        .l()?;
        let s: String = env.get_string(&JString::from(v))?.into();
        Ok(s.split(',')
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect())
    })
    .unwrap_or_default()
}

/// Shows Android's permission prompts for what's missing. The UI explains why first.
pub fn request_permissions() {
    with_context(|env, context| {
        call_static(env, context, "Permissions", "request", "()V", &[]).map(|_| ())
    });
}
