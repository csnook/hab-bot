package dev.habbot.reminders

import android.content.Context
import java.io.File

/** The Rust core, called from receivers without starting the Activity. */
object Native {
    init {
        System.loadLibrary("hab_bot_lib")
    }

    /**
     * Fires what is due and asks the alert engine what to show. Returns JSON:
     * `{alerts: [{occurrence_id, title, kind, style, overdue, priority, expires_at}],
     *   dismissed: [occurrence_id], next_wake: epochMillis or -1}`.
     * [dnd] is whether Android's Do Not Disturb is on.
     */
    @JvmStatic external fun tick(db: String, now: Long, dnd: Boolean, device: String): String

    /** What a notification button or swipe does: `done`, `skip`, `snooze` or `swipe`. */
    @JvmStatic external fun act(db: String, action: String, occurrenceId: String, now: Long): Boolean

    /** When to wake next, or -1. */
    @JvmStatic external fun nextWake(db: String, now: Long, device: String): Long

    /** The same file the Rust side opens in the Activity (`files_dir` in android.rs). */
    fun dbPath(context: Context): String = File(context.filesDir, "reminders.db").absolutePath

    fun deviceName(): String = android.os.Build.MODEL ?: "this phone"
}
