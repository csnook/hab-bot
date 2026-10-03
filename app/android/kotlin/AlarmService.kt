package dev.habbot.reminders

import android.app.Notification
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.media.AudioAttributes
import android.media.MediaPlayer
import android.media.RingtoneManager
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.VibrationEffect
import android.os.Vibrator

/**
 * The alarm, like the stock alarm clock: a foreground service that keeps the sound and
 * vibration going, looping, with the volume rising over 30 seconds. The full-screen alarm
 * screen comes from the notification's full-screen intent. What rings and when is decided
 * in Rust; this only plays.
 */
class AlarmService : Service() {
    private var player: MediaPlayer? = null
    private val handler = Handler(Looper.getMainLooper())
    private val ringing = linkedSetOf<String>()

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val id = intent?.getStringExtra(Notifier.EXTRA_OCCURRENCE)
        when (intent?.action) {
            ACTION_RING -> if (id != null) ring(id, intent)
            ACTION_STOP -> if (id != null) stop(id)
        }
        return START_NOT_STICKY
    }

    private fun ring(id: String, intent: Intent) {
        // The service must go foreground at once, with the same notification Notifier posts.
        val notification = Notifier.alarmNotification(
            this,
            id,
            intent.getStringExtra(Notifier.EXTRA_TITLE) ?: "",
            intent.getStringExtra(Notifier.EXTRA_PRIORITY) ?: "high",
            intent.getLongExtra(Notifier.EXTRA_DUE, 0),
        )
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(Notifier.notificationId(id), notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK)
        } else {
            startForeground(Notifier.notificationId(id), notification)
        }
        if (ringing.add(id) && ringing.size == 1) startSound()
    }

    private fun stop(id: String) {
        ringing.remove(id)
        if (ringing.isEmpty()) {
            stopSound()
            stopForeground(STOP_FOREGROUND_REMOVE)
            stopSelf()
        }
    }

    private fun startSound() {
        val attributes = AudioAttributes.Builder()
            .setUsage(AudioAttributes.USAGE_ALARM)
            .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
            .build()
        player = MediaPlayer().apply {
            setAudioAttributes(attributes)
            setDataSource(this@AlarmService, RingtoneManager.getDefaultUri(RingtoneManager.TYPE_ALARM))
            isLooping = true
            setVolume(0f, 0f)
            prepare()
            start()
        }
        // Rising in volume over 30 seconds.
        var step = 0
        val ramp = object : Runnable {
            override fun run() {
                step++
                val v = (step / RAMP_SECONDS.toFloat()).coerceAtMost(1f)
                player?.setVolume(v, v)
                if (v < 1f) handler.postDelayed(this, 1000)
            }
        }
        handler.postDelayed(ramp, 1000)

        @Suppress("DEPRECATION")
        val vibrator = getSystemService(Context.VIBRATOR_SERVICE) as Vibrator
        val pattern = longArrayOf(0, 800, 800)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            vibrator.vibrate(VibrationEffect.createWaveform(pattern, 0))
        } else {
            @Suppress("DEPRECATION") vibrator.vibrate(pattern, 0)
        }
    }

    private fun stopSound() {
        handler.removeCallbacksAndMessages(null)
        player?.run {
            stop()
            release()
        }
        player = null
        (getSystemService(Context.VIBRATOR_SERVICE) as Vibrator).cancel()
    }

    override fun onDestroy() {
        stopSound()
        super.onDestroy()
    }

    companion object {
        const val ACTION_RING = "dev.habbot.reminders.RING"
        const val ACTION_STOP = "dev.habbot.reminders.STOP_RINGING"
        const val RAMP_SECONDS = 30

        fun ring(context: Context, id: String, title: String, priority: String, dueAt: Long) {
            val intent = Intent(context, AlarmService::class.java)
                .setAction(ACTION_RING)
                .putExtra(Notifier.EXTRA_OCCURRENCE, id)
                .putExtra(Notifier.EXTRA_TITLE, title)
                .putExtra(Notifier.EXTRA_PRIORITY, priority)
                .putExtra(Notifier.EXTRA_DUE, dueAt)
            context.startForegroundService(intent)
        }

        fun stop(context: Context, id: String) {
            // Only an alarm that is ringing has anything to stop.
            if (!ringingIds.contains(id)) return
            context.startService(
                Intent(context, AlarmService::class.java).setAction(ACTION_STOP).putExtra(Notifier.EXTRA_OCCURRENCE, id),
            )
        }

        /** What the service has been asked to ring in this process; a hint to avoid needless starts. */
        val ringingIds: MutableSet<String> = java.util.Collections.synchronizedSet(mutableSetOf())
    }
}
