package dev.habbot.reminders

import android.app.AlarmManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.os.Build

object Alarms {
    private fun canExact(manager: AlarmManager) =
        Build.VERSION.SDK_INT < Build.VERSION_CODES.S || manager.canScheduleExactAlarms()

    private const val REQUEST = 1

    private fun pending(context: Context): PendingIntent =
        PendingIntent.getBroadcast(
            context,
            REQUEST,
            Intent(context, AlarmReceiver::class.java),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )

    /**
     * Registers the exact alarm for the next thing to do (epoch millis), or cancels it when
     * [at] is negative. The quieter priorities use `setExactAndAllowWhileIdle`. An [alarm]
     * (High and Maximum, and Medium once escalated) uses `setAlarmClock`, as the stock
     * alarm clock does, which fires through Doze and shows the alarm in the status bar.
     */
    fun scheduleAt(context: Context, at: Long, alarm: Boolean = false) {
        val manager = context.getSystemService(Context.ALARM_SERVICE) as AlarmManager
        val intent = pending(context)
        if (at < 0) {
            manager.cancel(intent)
            return
        }
        if (alarm && canExact(manager)) {
            val show = PendingIntent.getActivity(
                context,
                REQUEST,
                Intent(context, MainActivity::class.java),
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
            manager.setAlarmClock(AlarmManager.AlarmClockInfo(at, show), intent)
            return
        }
        val exact = Build.VERSION.SDK_INT < Build.VERSION_CODES.S || manager.canScheduleExactAlarms()
        if (exact) {
            manager.setExactAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, at, intent)
        } else {
            // "Alarms & reminders" isn't allowed yet: inexact, until the user grants it.
            manager.setAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, at, intent)
        }
    }
}
