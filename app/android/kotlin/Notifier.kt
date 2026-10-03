package dev.habbot.reminders

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.os.Build
import org.json.JSONArray

/** Fires due reminders through Rust and posts a notification with Done for each. */
object Notifier {
    private const val CHANNEL = "reminders"
    const val EXTRA_OCCURRENCE = "occurrence"

    fun fireDue(context: Context) {
        val opened = JSONArray(Native.fire(Native.dbPath(context), System.currentTimeMillis()))
        for (i in 0 until opened.length()) {
            val o = opened.getJSONObject(i)
            post(context, o.getString("id"), o.getString("title"))
        }
        Alarms.scheduleNext(context)
    }

    fun notificationId(occurrenceId: String): Int = occurrenceId.hashCode()

    private fun post(context: Context, occurrenceId: String, title: String) {
        val manager = context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            manager.createNotificationChannel(
                NotificationChannel(CHANNEL, "Reminders", NotificationManager.IMPORTANCE_HIGH),
            )
        }
        val flags = PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        val id = notificationId(occurrenceId)
        val done = PendingIntent.getBroadcast(
            context,
            id,
            Intent(context, DoneReceiver::class.java).putExtra(EXTRA_OCCURRENCE, occurrenceId),
            flags,
        )
        val open = PendingIntent.getActivity(
            context,
            0,
            Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
            flags,
        )
        val builder =
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) Notification.Builder(context, CHANNEL)
            else @Suppress("DEPRECATION") Notification.Builder(context)
        val notification = builder
            .setSmallIcon(context.applicationInfo.icon)
            .setContentTitle(title)
            .setContentText("Due now")
            .setContentIntent(open)
            .addAction(Notification.Action.Builder(null, "Done", done).build())
            .build()
        manager.notify(id, notification)
    }
}
