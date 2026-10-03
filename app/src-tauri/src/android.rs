//! The Android side of the Rust core: JNI exports that Kotlin calls from the alarm,
//! boot and notification receivers without starting the Activity, and calls the other
//! way into the Kotlin helpers.
//!
//! The exports open their own `Core` on the database file each time: Tauri's managed
//! state doesn't exist when Android starts the process for a receiver.

use hab_core::{AlertEngine, Core, SnoozeVia};
use jni::objects::{JClass, JObject, JString, JValue};
use jni::sys::{jboolean, jlong, jstring};
use jni::JNIEnv;

/// The user's name in the skeleton, as in `lib.rs`.
const USER: &str = "me";
const PACKAGE: &str = "dev.habbot.reminders";

/// Floating reminders follow the device's current zone.
fn sync_zone(core: &mut Core) {
    if let Ok(zone) = iana_time_zone::get_timezone() {
        core.set_zone(&zone);
    }
}

fn with_core<T>(db: &str, f: impl FnOnce(&mut Core) -> hab_core::Result<T>) -> Option<T> {
    let mut core = Core::open(db, USER).ok()?;
    sync_zone(&mut core);
    f(&mut core).ok()
}

fn string(env: &mut JNIEnv, s: &JString) -> String {
    env.get_string(s).map(Into::into).unwrap_or_default()
}

fn to_jstring(env: &mut JNIEnv, s: String) -> jstring {
    env.new_string(s)
        .map(|s| s.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

/// `Native.tick(db, now, dnd, device)`: fires what is due, then asks the alert engine what to
/// show. Returns JSON `{alerts, dismissed, next_wake}`, where `next_wake` is when to wake
/// next (epoch millis) or -1. `dnd` is Android's interruption filter.
#[no_mangle]
pub extern "system" fn Java_dev_habbot_reminders_Native_tick<'l>(
    mut env: JNIEnv<'l>,
    _class: JClass<'l>,
    db: JString<'l>,
    now: jlong,
    dnd: jboolean,
    device: JString<'l>,
) -> jstring {
    let db = string(&mut env, &db);
    let device = string(&mut env, &device);
    let json = with_core(&db, |core| {
        core.fire_due(now)?;
        let mut engine = AlertEngine::load(core, &device);
        let poll = engine.poll(core, now, dnd != 0)?;
        engine.save(core)?;
        let next = core.next_wake(&engine, now);
        // Alarms are registered with `setAlarmClock`, as the stock alarm clock does.
        let alarm = next.is_some_and(|at| engine.wake_is_alarm(core, at));
        Ok(serde_json::json!({
            "alerts": poll.alerts,
            "dismissed": poll.dismissed,
            "next_wake": next.unwrap_or(-1),
            "next_wake_alarm": alarm,
        }))
    })
    .unwrap_or_else(|| serde_json::json!({ "alerts": [], "dismissed": [], "next_wake": -1, "next_wake_alarm": false }));
    to_jstring(&mut env, json.to_string())
}

/// `Native.act(db, action, occurrenceId, now)`: what a notification button or swipe does.
/// `action` is `done`, `skip`, `acknowledge`, `snooze` (the button) or `swipe` (swiped away, which snoozes
/// for the priority's interval, recorded as made by swiping).
#[no_mangle]
pub extern "system" fn Java_dev_habbot_reminders_Native_act<'l>(
    mut env: JNIEnv<'l>,
    _class: JClass<'l>,
    db: JString<'l>,
    action: JString<'l>,
    occurrence_id: JString<'l>,
    now: jlong,
) -> jboolean {
    let db = string(&mut env, &db);
    let action = string(&mut env, &action);
    let id = string(&mut env, &occurrence_id);
    with_core(&db, |core| match action.as_str() {
        "done" => core.complete(&id, now),
        "skip" => core.skip(&id, None, now),
        // `skip:<note>` and `snooze_until:<epoch millis>`, from the alarm screen
        a if a.starts_with("skip:") => {
            let note = a["skip:".len()..].trim();
            core.skip(&id, (!note.is_empty()).then(|| note.to_string()), now)
        }
        a if a.starts_with("snooze_until:") => {
            let until = a["snooze_until:".len()..].parse().unwrap_or(0);
            core.snooze(&id, until, SnoozeVia::Button, now)
        }
        "acknowledge" => core.acknowledge(&id, now).map(|_| ()),
        "snooze" => core.snooze_default(&id, SnoozeVia::Button, now).map(|_| ()),
        "swipe" => core.snooze_default(&id, SnoozeVia::Swipe, now).map(|_| ()),
        _ => Ok(()),
    })
    .is_some() as jboolean
}

/// `Native.nextWake(db, now)`: when to wake next, or -1. Used after the app itself changes
/// something, such as a new reminder.
#[no_mangle]
pub extern "system" fn Java_dev_habbot_reminders_Native_nextWake<'l>(
    mut env: JNIEnv<'l>,
    _class: JClass<'l>,
    db: JString<'l>,
    now: jlong,
    device: JString<'l>,
) -> jlong {
    let db = string(&mut env, &db);
    let device = string(&mut env, &device);
    with_core(&db, |core| {
        let engine = AlertEngine::load(core, &device);
        Ok(core.next_wake(&engine, now))
    })
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

/// Has Kotlin fire what's due, update the notifications and register the next exact alarm.
/// Called after the app itself changes something, such as making a reminder.
pub fn refresh() {
    with_context(|env, context| {
        call_static(
            env,
            context,
            "Notifier",
            "run",
            "(Landroid/content/Context;)V",
            &[JValue::Object(context)],
        )
        .map(|_| ())
    });
}

/// The permissions the app still needs, as names (`notifications`, `alarms`).
pub fn missing_permissions(full_screen: bool) -> Vec<String> {
    with_context(|env, context| {
        let v = call_static(
            env,
            context,
            "Permissions",
            "missing",
            "(Landroid/content/Context;Z)Ljava/lang/String;",
            &[JValue::Object(context), JValue::Bool(full_screen as u8)],
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
