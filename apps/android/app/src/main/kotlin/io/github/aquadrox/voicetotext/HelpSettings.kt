package io.github.aquadrox.voicetotext

import android.content.Intent
import android.net.Uri
import android.widget.LinearLayout

/** Aide : comment dicter, import des réglages du PC, VoiceToText sur PC. */
class HelpSettings : SettingsPage() {
    override val section = Section.HELP
    override val subtitle = "Version ${BuildConfig.VERSION_NAME} · édition ${Updater.edition}"

    override fun LinearLayout.build() {
        card("Comment dicter", R.drawable.ic_mic).anchor("howto").apply {
            note("Clavier : maintenez le micro pour dicter, relâchez pour insérer. Appui court : mains libres, nouvel appui pour finir.")
            note("À droite du micro, la touche consigne (maintenue) : la consigne dictée s'applique au texte sélectionné (« rends ce texte plus formel », « traduis en anglais »).")
            note("À gauche du micro : le mode traduction. Tout à gauche : retour à votre clavier habituel.")
            if (BuildConfig.BUBBLE) {
                note("Bulle : toucher pour dicter, toucher encore pour insérer. Maintenir : consigne sur le texte sélectionné. Glisser : déplacer. Double appui : passer au clavier VoiceToText (Android 11 et plus).")
            }
        }

        card("Réglages du PC", R.drawable.ic_import).anchor("import").apply {
            note("Reprenez le dictionnaire, les snippets, les styles et le fournisseur LLM du PC. Sur PC : Réglages → Exporter…, puis copiez le fichier sur le téléphone. Les clés API ne sont jamais exportées.")
            secondary("Importer un fichier de réglages…") {
                startActivityForResult(
                    Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
                        addCategory(Intent.CATEGORY_OPENABLE)
                        type = "*/*"
                    },
                    IMPORT,
                )
            }
        }

        card("VoiceToText sur PC", R.drawable.ic_monitor).anchor("pc").apply {
            note("La même dictée sur Windows, gratuite : maintenez AltGr et parlez, le texte nettoyé s'écrit dans n'importe quelle application. Vos réglages passent d'un appareil à l'autre.")
            secondary("Découvrir VoiceToText pour PC") {
                startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(PROJECT)))
            }
        }
    }

    @Deprecated("API Activity simple, sans AndroidX")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        val uri: Uri = data?.data ?: return
        if (requestCode != IMPORT || resultCode != RESULT_OK) return
        val text = runCatching {
            contentResolver.openInputStream(uri)?.use { it.readBytes().toString(Charsets.UTF_8) }
        }.getOrNull()
        if (text == null || text.length > 1_000_000) {
            say("Fichier illisible.")
            return
        }
        val reply = org.json.JSONObject(Native.importSettings(text))
        val ok = reply.optJSONObject("ok") ?: run {
            say(reply.optString("error"))
            return
        }
        store.config = ok.getString("toml")
        val notes = ok.getJSONArray("notes")
        val extra = (0 until notes.length()).joinToString("") { "\n• ${notes.getString(it)}" }
        val host = Config.view(store)?.optJSONObject("llm")?.optString("host").orEmpty()
        val key = when {
            host.isEmpty() || store.hasKey(host) -> ""
            else -> "\nSaisissez la clé API de $host dans « Nettoyage du texte » : les clés ne sont jamais exportées."
        }
        say("✓ Réglages du PC importés : utilisés dès la prochaine dictée.$key$extra")
    }

    private companion object {
        const val IMPORT = 2
        const val PROJECT = "https://github.com/Aquadrox/VoiceToText"
    }
}
