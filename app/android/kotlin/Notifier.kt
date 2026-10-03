package dev.habbot.reminders

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.media.AudioAttributes
import android.media.RingtoneManager
import android.os.Build
import org.json.JSONObject

/**
 * Shows what the Rust core's alert engine says to show, as Android notifications. Kotlin
 * only carries events to and from Rust: which style, when and how often are decided there.
 */
object Notifier {
    const val EXTRA_OCCURRENCE = "occurrence"
    const val EXTRA_ACTION = "action"
    const val EXTRA_TITLE = "title"
    const val EXTRA_PRIORITY = "priority"
    const val EXTRA_DUE = "due"

    private const val CH_SILENT = "silent"
    private const val CH_GENTLE = "gentle"
    private const val CH_ALARM = "alarm"
    private const val GROUP_QUIET = "quiet"
    private const val SUMMARY_ID = 1

    /** Fires what's due, updates the notifications and registers the next exact alarm. */
    @JvmStatic
    fun run(context: Context) {
        val manager = context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        createChannels(manager)
        // The app reads Android's Do Not Disturb setting itself.
        val dnd = manager.currentInterruptionFilter != NotificationManager.INTERRUPTION_FILTER_ALL
        val result = JSONObject(
            Native.tick(Native.dbPath(context), System.currentTimeMillis(), dnd, Native.deviceName()),
        )
        val dismissed = result.getJSONArray("dismissed")
        for (i in 0 until dismissed.length()) {
            manager.cancel(id(dismissed.getString(i)))
            AlarmService.stop(context, dismissed.getString(i))
            AlarmService.ringingIds.remove(dismissed.getString(i))
        }
        val alerts = result.getJSONArray("alerts")
        for (i in 0 until alerts.length()) post(context, manager, alerts.getJSONObject(i))
        updateQuietSummary(context, manager)
        Alarms.scheduleAt(context, result.getLong("next_wake"), result.getBoolean("next_wake_alarm"))
    }

    /** A notification button or swipe: Rust acts, then the notifications catch up. */
    fun act(context: Context, action: String, occurrenceId: String) {
        Native.act(Native.dbPath(context), action, occurrenceId, System.currentTimeMillis())
        run(context)
    }

    fun notificationId(occurrenceId: String): Int = occurrenceId.hashCode().coerceAtLeast(2)
    private fun id(occurrenceId: String): Int = notificationId(occurrenceId)

    private fun createChannels(manager: NotificationManager) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        fun channel(id: String, name: String, importance: Int, sound: Boolean, alarm: Boolean = false) {
            val c = NotificationChannel(id, name, importance)
            if (!sound) c.setSound(null, null)
            if (alarm) {
                c.setSound(
                    RingtoneManager.getDefaultUri(RingtoneManager.TYPE_ALARM),
                    AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_ALARM).build(),
                )
                c.setBypassDnd(false) // Maximum asks for it with the alarm category and DND access
            }
            manager.createNotificationChannel(c)
        }
        // Silent: in the shade, with no sound or vibration.
        channel(CH_SILENT, "Quiet reminders", NotificationManager.IMPORTANCE_LOW, sound = false)
        // Gentle (and insistent, which repeats it): a pop-up with one sound and vibration.
        channel(CH_GENTLE, "Reminders", NotificationManager.IMPORTANCE_HIGH, sound = true)
        // Alarm: until the full-screen alarm exists, a loud pop-up in the alarm category.
        channel(CH_ALARM, "Alarms", NotificationManager.IMPORTANCE_HIGH, sound = true, alarm = true)
    }

    private fun post(context: Context, manager: NotificationManager, alert: JSONObject) {
        val occurrenceId = alert.getString("occurrence_id")
        // An occurrence that was ringing and is now quieter (acknowledged, snoozed) stops ringing.
        if (alert.getString("style") != "alarm") {
            AlarmService.stop(context, occurrenceId)
            AlarmService.ringingIds.remove(occurrenceId)
        }
        val title = alert.getString("title")
        val style = alert.getString("style")
        val priority = alert.getString("priority")
        val kind = alert.getString("kind")
        val quiet = priority == "minimum" || priority == "low"
        val unswipeable = priority == "high" || priority == "maximum"
        val flags = PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        val nid = id(occurrenceId)

        fun action(name: String, requestOffset: Int): PendingIntent = PendingIntent.getBroadcast(
            context,
            nid * 8 + requestOffset,
            Intent(context, ActionReceiver::class.java)
                .putExtra(EXTRA_OCCURRENCE, occurrenceId)
                .putExtra(EXTRA_ACTION, name),
            flags,
        )

        val channel = when (style) {
            "silent" -> CH_SILENT
            "alarm" -> CH_ALARM
            else -> CH_GENTLE
        }
        val builder =
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) Notification.Builder(context, channel)
            else @Suppress("DEPRECATION") Notification.Builder(context)
        val text = when {
            kind == "last_chance" -> "Last chance: expires soon"
            alert.getBoolean("overdue") -> "Overdue"
            else -> "Due now"
        }
        builder
            .setSmallIcon(context.applicationInfo.icon)
            .setContentTitle(if (kind == "last_chance") "Last chance: $title" else title)
            .setContentText(text)
            .setContentIntent(
                PendingIntent.getActivity(
                    context,
                    0,
                    Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
                    flags,
                ),
            )
            .addAction(Notification.Action.Builder(null, "Done", action("done", 0)).build())
            .addAction(Notification.Action.Builder(null, "Snooze", action("snooze", 1)).build())
        // Alarms carry Done · Snooze · Acknowledge; the others Done · Snooze · Skip.
        if (style == "alarm") {
            builder.addAction(Notification.Action.Builder(null, "Acknowledge", action("acknowledge", 2)).build())
        } else {
            builder.addAction(Notification.Action.Builder(null, "Skip", action("skip", 2)).build())
        }
        // Maximum gets through Do Not Disturb with the alarm category.
        if (priority == "maximum") builder.setCategory(Notification.CATEGORY_ALARM)
        if (style == "silent") builder.setSilent(true)
        // Repeats alert again; updates that don't (silent, a style change) don't make noise twice.
        builder.setOnlyAlertOnce(kind == "style_change" && style == "silent")
        if (unswipeable) {
            // High and Maximum can't be swiped away.
            builder.setOngoing(true)
        } else {
            // Swiping a notification away is a snooze for the priority's interval.
            builder.setDeleteIntent(action("swipe", 3))
        }
        // Minimum and Low gather into one collapsed group; Medium and above stand alone.
        if (quiet) builder.setGroup(GROUP_QUIET)
        if (style == "alarm") {
            ring(context, manager, occurrenceId, title, priority, alert.getLong("scheduled_at"))
            return
        }
        manager.notify(nid, builder.build())
    }

    /**
     * The alarm style: a foreground service plays the looping sound and vibration, and the
     * notification it shows carries a full-screen intent for the alarm screen. Without the
     * full-screen permission it is a large pop-up notification with the same buttons.
     */
    private fun ring(context: Context, manager: NotificationManager, id: String, title: String, priority: String, dueAt: Long) {
        AlarmService.ringingIds.add(id)
        AlarmService.ring(context, id, title, priority, dueAt)
    }

    /** The alarm's notification, also the service's foreground notification. */
    fun alarmNotification(context: Context, occurrenceId: String, title: String, priority: String, dueAt: Long): Notification {
        val flags = PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        val nid = notificationId(occurrenceId)
        fun action(name: String, offset: Int): PendingIntent = PendingIntent.getBroadcast(
            context,
            nid * 8 + offset,
            Intent(context, ActionReceiver::class.java)
                .putExtra(EXTRA_OCCURRENCE, occurrenceId)
                .putExtra(EXTRA_ACTION, name),
            flags,
        )
        val screen = PendingIntent.getActivity(
            context,
            nid * 8 + 4,
            Intent(context, AlarmActivity::class.java)
                .putExtra(EXTRA_OCCURRENCE, occurrenceId)
                .putExtra(EXTRA_TITLE, title)
                .putExtra(EXTRA_PRIORITY, priority)
                .putExtra(EXTRA_DUE, dueAt)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_NO_USER_ACTION),
            flags,
        )
        val builder =
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) Notification.Builder(context, CH_ALARM)
            else @Suppress("DEPRECATION") Notification.Builder(context)
        builder
            .setSmallIcon(context.applicationInfo.icon)
            .setContentTitle(title)
            .setContentText("Ringing")
            .setCategory(Notification.CATEGORY_ALARM)
            .setOngoing(true)
            .setContentIntent(screen)
            .addAction(Notification.Action.Builder(null, "Done", action("done", 0)).build())
            .addAction(Notification.Action.Builder(null, "Snooze", action("snooze", 1)).build())
            .addAction(Notification.Action.Builder(null, "Acknowledge", action("acknowledge", 2)).build())
        // Without the permission this stays a large pop-up (heads-up) notification.
        if (canUseFullScreen(context)) builder.setFullScreenIntent(screen, true)
        return builder.build()
    }

    fun canUseFullScreen(context: Context): Boolean {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.UPSIDE_DOWN_CAKE) return true
        return (context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager).canUseFullScreenIntent()
    }

    /** The "quiet reminders" group's summary line, shown while the group has members. */
    private fun updateQuietSummary(context: Context, manager: NotificationManager) {
        val members = manager.activeNotifications.filter {
            it.id != SUMMARY_ID && it.notification.group == GROUP_QUIET
        }
        if (members.isEmpty()) {
            manager.cancel(SUMMARY_ID)
            return
        }
        val line = "${members.size} quiet reminder${if (members.size == 1) "" else "s"}"
        val builder =
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) Notification.Builder(context, CH_SILENT)
            else @Suppress("DEPRECATION") Notification.Builder(context)
        manager.notify(
            SUMMARY_ID,
            builder
                .setSmallIcon(context.applicationInfo.icon)
                .setContentTitle(line)
                .setGroup(GROUP_QUIET)
                .setGroupSummary(true)
                .setSilent(true)
                .setStyle(Notification.InboxStyle().setSummaryText(line))
                .build(),
        )
    }
}
