package dev.habbot.reminders

import android.app.Activity
import android.app.AlertDialog
import android.app.TimePickerDialog
import android.graphics.Color
import android.os.Build
import android.os.Bundle
import android.view.Gravity
import android.view.ViewGroup
import android.view.WindowManager
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView
import java.text.DateFormat
import java.util.Calendar
import java.util.Date

/**
 * The full-screen alarm screen, shown over the lock screen with the screen on:
 * "Ringing · list · priority", the title, when it was due, a large Done, Snooze ▾,
 * Acknowledge and Skip…. Every button goes through Rust, then the alarm is over.
 */
class AlarmActivity : Activity() {
    private lateinit var occurrenceId: String

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O_MR1) {
            setShowWhenLocked(true)
            setTurnScreenOn(true)
        } else {
            @Suppress("DEPRECATION")
            window.addFlags(
                WindowManager.LayoutParams.FLAG_SHOW_WHEN_LOCKED or WindowManager.LayoutParams.FLAG_TURN_SCREEN_ON,
            )
        }
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)

        occurrenceId = intent.getStringExtra(Notifier.EXTRA_OCCURRENCE) ?: return finish()
        val title = intent.getStringExtra(Notifier.EXTRA_TITLE) ?: ""
        val priority = intent.getStringExtra(Notifier.EXTRA_PRIORITY) ?: "high"
        val due = intent.getLongExtra(Notifier.EXTRA_DUE, 0)

        fun text(value: String, size: Float, bold: Boolean = false) = TextView(this).apply {
            text = value
            textSize = size
            gravity = Gravity.CENTER
            setTextColor(Color.WHITE)
            if (bold) setTypeface(typeface, android.graphics.Typeface.BOLD)
            setPadding(0, 16, 0, 16)
        }
        fun button(label: String, size: Float = 18f, onClick: () -> Unit) = Button(this).apply {
            text = label
            textSize = size
            setOnClickListener { onClick() }
        }

        val column = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            gravity = Gravity.CENTER
            setBackgroundColor(Color.parseColor("#1b1b1f"))
            setPadding(48, 48, 48, 48)
        }
        column.addView(text("Ringing · Personal · ${priority.replaceFirstChar { it.uppercase() }}", 16f))
        column.addView(text(title, 32f, bold = true))
        column.addView(text("Due ${DateFormat.getDateTimeInstance(DateFormat.SHORT, DateFormat.SHORT).format(Date(due))}", 16f))
        val done = button("Done", 32f) { act("done") }
        column.addView(done, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 360))
        column.addView(button("Snooze ▾") { chooseSnooze() })
        column.addView(button("Acknowledge") { act("acknowledge") })
        column.addView(button("Skip…") { chooseSkip() })
        setContentView(column)
    }

    /** Everything goes through Rust; then the sound stops and the screen closes. */
    private fun act(action: String) {
        Notifier.act(applicationContext, action, occurrenceId)
        AlarmService.stop(applicationContext, occurrenceId)
        finish()
    }

    private fun chooseSnooze() {
        val now = System.currentTimeMillis()
        val choices = arrayOf("5 minutes", "10 minutes", "30 minutes", "Until…")
        AlertDialog.Builder(this).setTitle("Snooze").setItems(choices) { _, which ->
            when (which) {
                0 -> act("snooze_until:${now + 5 * 60_000}")
                1 -> act("snooze_until:${now + 10 * 60_000}")
                2 -> act("snooze_until:${now + 30 * 60_000}")
                else -> chooseTime()
            }
        }.show()
    }

    /** "Until…": the next time the clock shows the chosen time. */
    private fun chooseTime() {
        val now = Calendar.getInstance()
        TimePickerDialog(this, { _, hour, minute ->
            val at = Calendar.getInstance().apply {
                set(Calendar.HOUR_OF_DAY, hour)
                set(Calendar.MINUTE, minute)
                set(Calendar.SECOND, 0)
                if (timeInMillis <= now.timeInMillis) add(Calendar.DAY_OF_YEAR, 1)
            }
            act("snooze_until:${at.timeInMillis}")
        }, now.get(Calendar.HOUR_OF_DAY), now.get(Calendar.MINUTE), true).show()
    }

    private fun chooseSkip() {
        val note = EditText(this).apply { hint = "Note (optional)" }
        AlertDialog.Builder(this).setTitle("Skip").setView(note)
            .setPositiveButton("Skip") { _, _ -> act("skip:${note.text}") }
            .setNegativeButton("Cancel", null)
            .show()
    }
}
