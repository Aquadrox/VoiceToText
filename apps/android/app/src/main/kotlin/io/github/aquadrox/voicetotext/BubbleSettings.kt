package io.github.aquadrox.voicetotext

import android.content.Intent
import android.net.Uri
import android.provider.Settings
import android.util.TypedValue
import android.view.View
import android.widget.LinearLayout
import android.widget.RadioButton
import android.widget.RadioGroup
import android.widget.TextView

/**
 * Bulle de dictée. Édition bulle : état, activation et options ; édition
 * clavier : ce que l'édition bulle apporte, et où la trouver.
 */
class BubbleSettings : SettingsPage() {
    override val section = Section.BUBBLE
    override val subtitle = "Dicter sans changer de clavier"

    private var state: TextView? = null
    private var stateButtons: View? = null
    private var lastStop: TextView? = null
    private var historySummary: TextView? = null
    private val check = Runnable { if (!isDestroyed) refreshState() }

    override fun LinearLayout.build() {
        if (!BuildConfig.BUBBLE) {
            card("Édition bulle", R.drawable.ic_message_circle).anchor("bubble_edition").apply {
                note("Gardez votre clavier habituel : une bulle flottante apparaît quand vous écrivez. Toucher pour dicter, toucher encore pour insérer.")
                note("Elle est dans l'édition « bulle » de VoiceToText (APK android-bulle de la même version) : même application, vos réglages, modèles et clés sont gardés.")
                primary("Télécharger l'édition bulle") {
                    startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(RELEASES)))
                }
            }
            return
        }

        card("État", R.drawable.ic_list_checks).anchor("bubble_status").apply {
            state = note("")
            stateButtons = row {
                secondary("Infos de l'appli") {
                    startActivity(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:$packageName")))
                }
                secondary("Accessibilité") { BubbleStatus.openSettings(this@BubbleSettings) }
            }
            more(
                "1. Android 13 et plus : Infos de l'application → ⋮ (en haut à droite) → « Autoriser les paramètres restreints » (obligatoire pour une application installée hors Play Store).\n" +
                    "2. Accessibilité → Applications installées → « VoiceToText : bulle de dictée » → activer.\n" +
                    "3. Xiaomi, Samsung… : autorisez le démarrage automatique de VoiceToText ; dans Infos de l'application, mettez la batterie sur « Pas de restriction » ; dans les applications récentes, verrouillez VoiceToText (appui long sur sa carte → cadenas). Sinon le téléphone arrête la bulle, et Android ne la relance pas seul.\n\n" +
                    "Bulle arrêtée : désactivez-la puis réactivez-la dans l'accessibilité. Inutile de forcer l'arrêt de l'application.",
                "Comment l'activer (une seule fois)",
            )
            if (BubbleStatus.xiaomi) {
                secondary("Démarrage automatique (Xiaomi)") {
                    if (BubbleStatus.openAutostart(this@BubbleSettings)) {
                        say("Activez VoiceToText dans la liste « Démarrage automatique ».")
                    } else {
                        say("Écran introuvable sur ce téléphone : cherchez « démarrage automatique » dans les paramètres.")
                    }
                }
            }
            lastStop = note("").apply { visibility = View.GONE }
        }

        card("Comportement", R.drawable.ic_message_circle).apply {
            toggle(
                "Seulement avec le clavier ouvert",
                "Sinon la bulle apparaît dès qu'un champ de texte est sélectionné.",
                store.bubbleWithKeyboard,
            ) {
                store.bubbleWithKeyboard = it
                saved(if (it) "✓ La bulle n'apparaîtra qu'avec le clavier" else "✓ La bulle apparaîtra sur tout champ de texte")
            }.anchor("bubble_keyboard")
            toggle(
                "Insérer en collant",
                "Si le texte n'apparaît pas dans une application. Remplace le contenu du presse-papier.",
                store.bubblePaste,
            ) {
                store.bubblePaste = it
                saved(if (it) "✓ Texte inséré en collant" else "✓ Texte inséré directement")
            }.anchor("bubble_paste")
            toggle(
                "Traduire les dictées",
                "Comme le mode traduction du clavier, vers la langue choisie dans Dictée.",
                store.translate,
            ) {
                store.translate = it
                saved(if (it) "✓ Les dictées de la bulle seront traduites" else "✓ Traduction de la bulle coupée")
            }.anchor("bubble_translate")
        }

        card("Apparence", R.drawable.ic_dot).apply {
            text("Taille de la bulle", 16f).anchor("bubble_size").setPadding(0, dp(8), 0, 0)
            addView(RadioGroup(this@BubbleSettings).apply {
                orientation = RadioGroup.HORIZONTAL
                listOf("Petite", "Moyenne", "Grande").forEachIndexed { size, label ->
                    addView(RadioButton(this@BubbleSettings).apply {
                        id = View.generateViewId()
                        text = label
                        setTextColor(getColor(R.color.fg))
                        setTextSize(TypedValue.COMPLEX_UNIT_SP, 15f)
                        setPadding(dp(4), dp(10), dp(16), dp(10))
                        isChecked = size == store.bubbleSize
                        setOnCheckedChangeListener { _, checked ->
                            if (checked) {
                                store.bubbleSize = size
                                saved("✓ Bulle ${label.lowercase()} : à sa prochaine apparition")
                            }
                        }
                    })
                }
            })
            secondary("Remettre la bulle à sa place") {
                store.bubbleX = -1
                store.bubbleY = -1
                saved("✓ La bulle reprendra sa place à sa prochaine apparition")
            }.anchor("bubble_reset")
        }

        card("Confidentialité", R.drawable.ic_lock).apply {
            note("La bulle n'apparaît jamais sur un champ de mot de passe. Contrairement au clavier, elle ne voit pas la navigation privée : vous pouvez ne pas garder ses dictées.")
            historySummary = navRow("Historique de la bulle") { Section.HISTORY.open(this@BubbleSettings, "bubble_history") }
        }
    }

    override fun onResume() {
        super.onResume()
        // Retour de l'accessibilité ou de l'historique : état à jour.
        if (BuildConfig.BUBBLE) refreshState()
        historySummary?.summary(if (store.bubbleHistory) "Dictées de la bulle gardées" else "Dictées de la bulle non gardées")
    }

    private fun refreshState() {
        val state = state ?: return
        main.removeCallbacks(check)
        val now = runCatching { BubbleStatus.state(this) }.getOrDefault(BubbleStatus.State.DISABLED)
        if (now == BubbleStatus.State.STOPPED) BubbleStatus.recordStop(this, store)
        // Application tout juste lancée : Android reconnecte peut-être encore la bulle.
        if (now == BubbleStatus.State.STARTING) main.postDelayed(check, BubbleStatus.SETTLE_MS)
        state.text = when (now) {
            BubbleStatus.State.DISABLED -> "Pas encore activée : suivez « Comment l'activer » ci-dessous."
            BubbleStatus.State.RUNNING -> "✓ Activée : elle apparaît quand vous écrivez."
            BubbleStatus.State.STARTING -> "Démarrage…"
            BubbleStatus.State.STOPPED -> "⚠ Activée mais arrêtée par le téléphone : désactivez-la puis réactivez-la dans l'accessibilité."
        }
        stateButtons?.visibility = if (now == BubbleStatus.State.RUNNING || now == BubbleStatus.State.STARTING) View.GONE else View.VISIBLE
        val last = store.bubbleStop.takeIf { it.isNotEmpty() }
        lastStop?.apply {
            text = last?.let { "Dernier arrêt de la bulle par le téléphone : $it." }.orEmpty()
            visibility = if (last == null) View.GONE else View.VISIBLE
        }
    }

    companion object {
        /** Page des versions publiées (APK des deux éditions). */
        const val RELEASES = "https://github.com/Aquadrox/VoiceToText/releases/latest"
    }
}
