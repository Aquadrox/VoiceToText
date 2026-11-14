package io.github.aquadrox.voicetotext

import android.content.Intent
import android.widget.LinearLayout
import org.json.JSONObject

/** Historique : garder ou non les dictées, et les consulter. */
class HistorySettings : SettingsPage() {
    override val section = Section.HISTORY
    override val subtitle = "Vos dictées restent sur le téléphone"

    override fun LinearLayout.build() {
        val kept = Config.view(store)?.optBoolean("history") ?: true
        card().apply {
            toggle(
                "Garder les dictées",
                "Retrouvez et copiez vos dernières dictées. Elles ne quittent pas le téléphone ; rien n'est gardé en navigation privée avec le clavier.",
                kept,
            ) { on ->
                saveSettings(
                    JSONObject().put("history", on),
                    if (on) "✓ Les prochaines dictées seront gardées" else "✓ Les prochaines dictées ne seront plus gardées",
                )
            }.anchor("history")
            if (BuildConfig.BUBBLE) {
                toggle(
                    "Garder les dictées de la bulle",
                    "La bulle ne voit pas la navigation privée : coupez ceci pour qu'aucune de ses dictées ne soit gardée.",
                    store.bubbleHistory,
                ) {
                    store.bubbleHistory = it
                    saved(if (it) "✓ Dictées de la bulle gardées" else "✓ Dictées de la bulle non gardées")
                }.anchor("bubble_history")
            }
            divider()
            navRow("Voir l'historique", "Rechercher, copier, tout effacer") {
                startActivity(Intent(this@HistorySettings, HistoryActivity::class.java))
            }
        }.anchor("history_view")
    }
}
