package io.github.aquadrox.voicetotext

import android.app.Activity
import android.app.AlertDialog
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.view.View
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
import android.window.OnBackInvokedDispatcher
import org.json.JSONObject
import java.text.Normalizer

/*
 * Réglages rangés par catégorie : l'accueil en donne la liste (avec l'état
 * actuel de chacune) et une recherche ; chaque catégorie a sa page.
 */

/** Catégorie de réglages, dans l'ordre de l'accueil (les plus utilisées d'abord). */
enum class Section(val title: String, val icon: Int, val page: Class<out Activity>) {
    DICTATION("Dictée", R.drawable.ic_mic, DictationSettings::class.java),
    CLEANUP("Nettoyage du texte", R.drawable.ic_spell_check_2, CleanupSettings::class.java),
    VOCABULARY("Dictionnaire et snippets", R.drawable.ic_book_a, VocabularyActivity::class.java),
    BUBBLE("Bulle de dictée", R.drawable.ic_message_circle, BubbleSettings::class.java),
    HISTORY("Historique", R.drawable.ic_history, HistorySettings::class.java),
    UPDATES("Mises à jour", R.drawable.ic_refresh_cw, UpdatesSettings::class.java),
    HELP("Aide et réglages du PC", R.drawable.ic_circle_help, HelpSettings::class.java),

    /** Réglages de l'écran Réunions : trouvés par la recherche, pas dans la liste. */
    MEETINGS("Réunions", R.drawable.ic_audio_lines, MeetingsActivity::class.java),
    ;

    /** Ouvre la page ; `focus` : réglage à montrer (recherche). */
    fun open(context: Context, focus: String? = null) {
        context.startActivity(Intent(context, page).putExtra(SettingsPage.FOCUS, focus))
    }
}

/**
 * Un réglage trouvable par la recherche : `key` repère sa place dans la page,
 * `words` : synonymes et mots qu'un utilisateur taperait. `bubble` : seulement
 * dans l'édition bulle (true), seulement dans l'édition clavier (false).
 */
class Entry(val section: Section, val key: String, val title: String, val words: String = "", val bubble: Boolean? = null)

object SettingsIndex {
    val entries = listOf(
        Entry(Section.DICTATION, "model", "Modèle de transcription", "whisper parakeet télécharger précision vitesse voix reconnaissance"),
        Entry(Section.DICTATION, "keyboard", "Clavier VoiceToText", "activer choisir changer de clavier méthode de saisie"),
        Entry(Section.DICTATION, "translation", "Langue de traduction", "traduire anglais espagnol allemand mode traduction langue"),
        Entry(Section.DICTATION, "voice_commands", "Commandes vocales", "nouvelle ligne annuler la dernière dictée commande épeler lettres"),
        Entry(Section.CLEANUP, "llm", "Nettoyer le texte avec un LLM", "ponctuation majuscules correction ia intelligence artificielle brut"),
        Entry(Section.CLEANUP, "provider", "Fournisseur du LLM", "deepseek openai mistral glm openrouter ollama local serveur"),
        Entry(Section.CLEANUP, "key", "Clé API", "token jeton mot de passe compte"),
        Entry(Section.CLEANUP, "test", "Tester les réglages du LLM", "essai vérifier connexion erreur"),
        Entry(Section.CLEANUP, "advanced", "Adresse de l'API et modèle LLM", "url serveur avancé base modèle"),
        Entry(Section.CLEANUP, "prompts", "Prompts système", "consigne command mode instructions nettoyage avancé"),
        Entry(Section.CLEANUP, "keys", "Clés enregistrées", "supprimer remplacer clé api fournisseurs"),
        Entry(Section.VOCABULARY, "", "Dictionnaire", "mots noms propres orthographe jargon sigles vocabulaire"),
        Entry(Section.VOCABULARY, "", "Snippets", "raccourcis texte fixe signature déclencheur"),
        Entry(Section.BUBBLE, "bubble_status", "Activer la bulle", "accessibilité arrêtée relancer xiaomi démarrage automatique batterie", bubble = true),
        Entry(Section.BUBBLE, "bubble_keyboard", "Bulle seulement avec le clavier ouvert", "afficher apparaître", bubble = true),
        Entry(Section.BUBBLE, "bubble_paste", "Insérer en collant", "presse-papier coller texte n'apparaît pas", bubble = true),
        Entry(Section.BUBBLE, "bubble_translate", "Traduire les dictées de la bulle", "traduction langue", bubble = true),
        Entry(Section.BUBBLE, "bubble_size", "Taille de la bulle", "petite moyenne grande apparence", bubble = true),
        Entry(Section.BUBBLE, "bubble_reset", "Remettre la bulle à sa place", "position déplacer", bubble = true),
        Entry(Section.BUBBLE, "bubble_edition", "Bulle de dictée", "édition bulle flottante sans changer de clavier", bubble = false),
        Entry(Section.HISTORY, "history", "Garder les dictées", "historique confidentialité vie privée effacer", bubble = null),
        Entry(Section.HISTORY, "bubble_history", "Garder les dictées de la bulle", "historique navigation privée confidentialité", bubble = true),
        Entry(Section.HISTORY, "history_view", "Voir l'historique", "anciennes dictées copier retrouver"),
        Entry(Section.UPDATES, "updates_auto", "Vérifier les mises à jour automatiquement", "nouvelle version"),
        Entry(Section.UPDATES, "updates_check", "Vérifier maintenant", "mise à jour installer nouvelle version"),
        Entry(Section.UPDATES, "updates_notes", "Nouveautés de chaque version", "notes de version changelog historique des versions"),
        Entry(Section.HELP, "howto", "Comment dicter", "aide mode d'emploi mains libres consigne micro maintenir"),
        Entry(Section.HELP, "import", "Importer les réglages du PC", "exporter fichier windows transférer"),
        Entry(Section.HELP, "pc", "VoiceToText sur PC", "windows ordinateur altgr"),
        Entry(Section.MEETINGS, "", "Réunions", "enregistrer cours conférence compte rendu transcription audio intervenants"),
    )

    /** Réglages dont le titre, la catégorie ou les synonymes ont un mot commençant par chaque mot tapé. */
    fun search(query: String, bubble: Boolean): List<Entry> {
        val words = fold(query).split(' ').filter { it.isNotEmpty() }
        if (words.isEmpty()) return emptyList()
        return entries
            .filter { it.bubble == null || it.bubble == bubble }
            .filter { entry ->
                // Début de mot : « cle » trouve « clé », pas « déclencheur ».
                val haystack = fold("${entry.title} ${entry.section.title} ${entry.words}").split(' ')
                words.all { word -> haystack.any { it.startsWith(word) } }
            }
            // Titre qui commence par la recherche d'abord, puis titre qui la contient.
            .sortedBy { entry ->
                val title = fold(entry.title)
                when {
                    title.startsWith(words[0]) -> 0
                    words.all { word -> title.split(' ').any { it.startsWith(word) } } -> 1
                    else -> 2
                }
            }
    }

    /** Minuscules sans accents ni ponctuation : « Clé » trouve « cle ». */
    fun fold(text: String): String =
        Normalizer.normalize(text.lowercase(), Normalizer.Form.NFD)
            .replace(Regex("\\p{Mn}+"), "")
            .replace(Regex("[^a-z0-9]+"), " ")
            .trim()
}

/** Lecture et enregistrement de config.toml par le moteur Rust. */
object Config {
    /** Réglages affichés (JSON du moteur), ou null si config.toml est illisible. */
    fun view(store: Store): JSONObject? = JSONObject(Native.settings(store.config, store.modelsDir)).optJSONObject("ok")

    /**
     * Enregistre les champs de `change` ; les autres gardent leur valeur
     * actuelle. Renvoie l'erreur à afficher, ou null si c'est enregistré.
     */
    fun save(store: Store, change: JSONObject): String? {
        val reply = JSONObject(Native.settings(store.config, store.modelsDir))
        val view = reply.optJSONObject("ok") ?: return reply.optString("error")
        val llm = view.getJSONObject("llm")
        val update = JSONObject()
            .put("llm_enabled", llm.getBoolean("enabled"))
            .put("base_url", llm.getString("base_url"))
            .put("llm_model", llm.getString("model"))
            .put("model", view.getString("model"))
            .put("voice_commands", view.getBoolean("voice_commands"))
            .put("translation", view.getString("translation"))
            .put("history", view.getBoolean("history"))
        for (key in change.keys()) update.put(key, change.get(key))
        val saved = JSONObject(Native.updateSettings(store.config, update.toString()))
        if (saved.has("error")) return saved.getString("error")
        store.config = saved.getString("ok")
        return null
    }
}

/**
 * Page d'une catégorie : en-tête avec retour, message d'état, contenu.
 * Les interrupteurs et les choix s'enregistrent dès qu'on les touche, avec
 * une confirmation qui dit ce que cela change ; seul le formulaire du LLM
 * (adresse, modèle, clé, à tester ensemble) a un bouton « Enregistrer ».
 */
abstract class SettingsPage : Activity() {
    protected val store by lazy { Store(this) }
    protected val main = Handler(Looper.getMainLooper())
    protected lateinit var message: TextView
    private val anchors = mutableMapOf<String, View>()

    protected abstract val section: Section
    protected open val subtitle: String = ""

    /** Barre d'actions fixée en bas ; null : pas de barre. */
    protected open val bar: (LinearLayout.() -> Unit)? = null

    protected abstract fun LinearLayout.build()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        screen(bar = bar) {
            header(section.title, subtitle, onBack = { leave() })
            message = notice()
            build()
        }
        // Venu de la recherche : le réglage cherché est montré.
        intent.getStringExtra(FOCUS)?.let { key -> anchors[key]?.let { view -> main.postDelayed({ view.reveal() }, 250) } }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            onBackInvokedDispatcher.registerOnBackInvokedCallback(OnBackInvokedDispatcher.PRIORITY_DEFAULT) { leave() }
        }
    }

    @Deprecated("Android 12 et moins ; ensuite : onBackInvokedDispatcher")
    override fun onBackPressed() = leave()

    /** Retour : `canLeave` peut d'abord demander quoi faire des modifications. */
    protected fun leave() {
        if (canLeave()) finish()
    }

    protected open fun canLeave(): Boolean = true

    /** Repère un réglage pour la recherche. */
    protected fun <T : View> T.anchor(key: String): T = also { anchors[key] = it }

    /** Message en haut de la page (erreur, consigne) ; la page remonte pour le montrer. */
    protected fun say(text: String) {
        message.say(text)
        if (text.isNotEmpty()) message.reveal()
    }

    /** Confirmation brève après un enregistrement : ce qui est fait et ce que cela change. */
    protected fun saved(text: String) {
        message.say("")
        Toast.makeText(this, text, Toast.LENGTH_SHORT).show()
    }

    /** Enregistre une partie des réglages ; confirmation, ou erreur affichée. */
    protected fun saveSettings(change: JSONObject, confirmation: String): Boolean {
        val error = Config.save(store, change)
        if (error != null) {
            say("Non enregistré : $error")
            return false
        }
        saved(confirmation)
        return true
    }

    /** Question avant de quitter avec des modifications non enregistrées. */
    protected fun askSave(save: () -> Boolean) {
        AlertDialog.Builder(this)
            .setTitle("Enregistrer les modifications ?")
            .setMessage("Les changements de cette page ne sont pas encore enregistrés.")
            .setPositiveButton("Enregistrer") { _, _ -> if (save()) finish() }
            .setNegativeButton("Ne pas enregistrer") { _, _ -> finish() }
            .setNeutralButton("Annuler", null)
            .show()
    }

    override fun onDestroy() {
        main.removeCallbacksAndMessages(null)
        super.onDestroy()
    }

    companion object {
        const val FOCUS = "focus"
    }
}
