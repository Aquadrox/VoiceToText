package io.github.aquadrox.voicetotext

import android.app.Activity
import android.graphics.Typeface
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView

/** Petits éléments d'interface communs aux écrans (sans bibliothèque). */
fun Activity.dp(value: Int) = (value * resources.displayMetrics.density).toInt()

fun LinearLayout.title(text: String, size: Float = 18f) = TextView(context).apply {
    this.text = text
    textSize = size
    setTypeface(typeface, Typeface.BOLD)
    setPadding(0, (24 * resources.displayMetrics.density).toInt(), 0, 0)
}.also { addView(it) }

fun LinearLayout.note(text: String) = TextView(context).apply {
    this.text = text
    alpha = 0.8f
    setPadding(0, 8, 0, 8)
}.also { addView(it) }

fun LinearLayout.button(text: String, action: () -> Unit) = Button(context).apply {
    this.text = text
    setOnClickListener { action() }
}.also { addView(it) }

fun LinearLayout.field(hint: String) = EditText(context).apply {
    this.hint = hint
    isSingleLine = true
}.also { addView(it) }

/** Colonne d'écran avec marges. */
fun Activity.column() = LinearLayout(this).apply {
    orientation = LinearLayout.VERTICAL
    setPadding(dp(20), dp(16), dp(20), dp(32))
}
