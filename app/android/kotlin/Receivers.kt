package dev.habbot.reminders

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/** The exact alarm went off. Runs with the app closed: no Activity, no webview. */
class AlarmReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        Notifier.run(context)
    }
}

/** A notification's Done, Snooze or Skip, or its swipe: Rust acts without opening the app. */
class ActionReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val id = intent.getStringExtra(Notifier.EXTRA_OCCURRENCE) ?: return
        val action = intent.getStringExtra(Notifier.EXTRA_ACTION) ?: return
        Notifier.act(context, action, id)
    }
}

/**
 * Alarms are cleared by a reboot, so they're registered again. The same covers the clock or
 * time zone changing, an update, and the exact-alarm permission being granted. Anything that
 * came due in the meantime fires now.
 */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        Notifier.run(context)
    }
}
