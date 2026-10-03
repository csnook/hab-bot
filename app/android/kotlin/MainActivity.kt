package dev.habbot.reminders

import android.os.Bundle
import app.tauri.TauriActivity

class MainActivity : TauriActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        current = this
    }

    override fun onDestroy() {
        if (current === this) current = null
        super.onDestroy()
    }

    companion object {
        /** The running Activity, for permission prompts. */
        @Volatile
        @JvmStatic
        var current: MainActivity? = null
    }
}
