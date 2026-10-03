package dev.habbot.reminders

import android.Manifest
import android.app.AlarmManager
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.provider.Settings

/**
 * Notifications (Android 13 and later) and Alarms & reminders (Android 12 and later).
 * The UI explains why before calling [request]; the full first-start flow comes later.
 */
object Permissions {
    private const val NOTIFICATIONS_REQUEST = 100

    /** Comma-separated names of what's still missing: `notifications`, `alarms`. Called from Rust. */
    @JvmStatic
    fun missing(context: Context): String {
        val missing = mutableListOf<String>()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        ) missing += "notifications"
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S &&
            !(context.getSystemService(Context.ALARM_SERVICE) as AlarmManager).canScheduleExactAlarms()
        ) missing += "alarms"
        return missing.joinToString(",")
    }

    /** Shows Android's prompt for each missing permission. Called from Rust. */
    @JvmStatic
    fun request() {
        val activity = MainActivity.current ?: return
        activity.runOnUiThread {
            val missing = missing(activity).split(",")
            if ("notifications" in missing && Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                activity.requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), NOTIFICATIONS_REQUEST)
            }
            if ("alarms" in missing && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                activity.startActivity(
                    Intent(Settings.ACTION_REQUEST_SCHEDULE_EXACT_ALARM, Uri.parse("package:${activity.packageName}")),
                )
            }
        }
    }
}
