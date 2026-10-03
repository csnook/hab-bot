package dev.habbot.reminders

import android.app.NotificationManager
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/** The exact alarm went off. Runs with the app closed: no Activity, no webview. */
class AlarmReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        Notifier.fireDue(context)
    }
}

/** The notification's Done button: completes through Rust without opening the app. */
class DoneReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val id = intent.getStringExtra(Notifier.EXTRA_OCCURRENCE) ?: return
        Native.complete(Native.dbPath(context), id, System.currentTimeMillis())
        (context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager)
            .cancel(Notifier.notificationId(id))
    }
}

/**
 * Alarms are cleared by a reboot, so they're registered again. The same covers the clock or
 * time zone changing, an update, and the exact-alarm permission being granted. Anything that
 * came due in the meantime fires now.
 */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        Notifier.fireDue(context)
    }
}
