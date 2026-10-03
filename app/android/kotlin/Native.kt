package dev.habbot.reminders

import android.content.Context
import java.io.File

/** The Rust core, called from receivers without starting the Activity. */
object Native {
    init {
        System.loadLibrary("hab_bot_lib")
    }

    /** Fires what is due. Returns a JSON array of `{id, title}` for the occurrences opened. */
    @JvmStatic external fun fire(db: String, now: Long): String

    /** Completes an open occurrence. */
    @JvmStatic external fun complete(db: String, occurrenceId: String, now: Long): Boolean

    /** When the next unfired reminder comes due, or -1. */
    @JvmStatic external fun nextDue(db: String, now: Long): Long

    /** The same file the Rust side opens in the Activity (`files_dir` in android.rs). */
    fun dbPath(context: Context): String = File(context.filesDir, "reminders.db").absolutePath
}
