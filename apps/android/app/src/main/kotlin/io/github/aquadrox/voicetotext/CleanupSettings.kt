package io.github.aquadrox.voicetotext

import android.content.Intent
import android.graphics.Typeface
import android.text.Editable
import android.text.InputType
import android.text.TextWatcher
import android.util.TypedValue
import android.view.View
import android.widget.AdapterView
import android.widget.ArrayAdapter
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.Spinner
import android.widget.Switch
import android.widget.TextView
import org.json.JSONArray
import org.json.JSONObject
import java.util.concurrent.Executors

/**
 * Nettoyage du texte par un LLM : marche/arrêt, fournisseur et clé (réglages
 * de base), adresse, modèle et prompts (avancés, repliés), clés enregistrées.
 */
class CleanupSettings : SettingsPage() {
    override val section = Section.CLEANUP
    override val subtitle = "Ponctuation et mise en forme de la dictée"
    override val bar: (LinearLayout.() -> Unit) = { primary("Enregistrer le fournisseur") { saveForm() } }

    private lateinit var llmEnabled: Switch
    private lateinit var provider: Spinner
    private lateinit var providerNote: TextView
    private lateinit var advanced: Fold
    private lateinit var baseUrl: EditText
    private lateinit var llmModel: EditText
    private lateinit var apiKey: EditText
    private lateinit var keyStatus: TextView
    private lateinit var formMessage: TextView
    private lateinit var keysList: LinearLayout
    private lateinit var keysEmpty: TextView
    private lateinit var keysMessage: TextView
    private lateinit var testButton: Button
    private lateinit var testResult: TextView
    private lateinit var testModels: FlowLayout
    private lateinit var promptsStatus: TextView

    /** Tests des réglages LLM (réseau). */
    private val tests = Executors.newSingleThreadExecutor()

    /** Préréglages des fournisseurs (moteur Rust), dans l'ordre du menu. */
    private var providers = emptyList<JSONObject>()

    /** Adresse changée par le code : pas de réaction en retour. */
    private var syncing = false

    /**
     * Position du menu posée par le code. Android signale la sélection plus
     * tard (au prochain affichage) : elle est reconnue ici et ignorée.
     */
    private var shownProvider = -1

    /** Adresse et modèle enregistrés : pour savoir si le formulaire a changé. */
    private var savedUrl = ""
    private var savedModel = ""

    override fun LinearLayout.build() {
        card().anchor("llm").apply {
            llmEnabled = toggle(
                "Nettoyer le texte avec un LLM",
                "Ponctuation, majuscules, hésitations retirées. Le texte transcrit est envoyé au fournisseur choisi ci-dessous ; désactivé, le texte brut est inséré.",
            ) { on -> switchCleanup(on) }
        }

        card("Fournisseur", R.drawable.ic_spell_check_2).anchor("provider").apply {
            note("Le service qui nettoie le texte. Choisir un fournisseur remplit son adresse et son modèle.")
            provider = Spinner(this@CleanupSettings).also { addView(it) }
            providerNote = note("")
            apiKey = keyField("Clé API (jamais affichée)").anchor("key")
            keyStatus = note("")
            advanced = fold("Adresse et modèle (avancé)") {
                note("À changer seulement pour un autre modèle ou un serveur à vous (Ollama, adresse libre).")
                baseUrl = field("Adresse de l'API (ex. https://api.deepseek.com)")
                llmModel = field("Modèle (ex. deepseek-flash)")
            }
            advanced.toggle.anchor("advanced")
            testButton = secondary("Tester avant d'enregistrer") { testForm() }.anchor("test")
            testResult = notice()
            testModels = FlowLayout(this@CleanupSettings).apply { visibility = View.GONE }
            addView(testModels, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply {
                topMargin = dp(6)
            })
            formMessage = notice()
            baseUrl.addTextChangedListener(object : TextWatcher {
                override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) {}
                override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) {}
                override fun afterTextChanged(s: Editable?) {
                    if (!syncing) refreshKeyTarget(selectProvider = true)
                }
            })
            provider.onItemSelectedListener = object : AdapterView.OnItemSelectedListener {
                override fun onItemSelected(parent: AdapterView<*>?, view: View?, position: Int, id: Long) {
                    if (position == shownProvider) return
                    shownProvider = position
                    chooseProvider(position)
                }
                override fun onNothingSelected(parent: AdapterView<*>?) {}
            }
        }

        card("Avancé", R.drawable.ic_list_checks).anchor("prompts").apply {
            promptsStatus = navRow("Prompts système") {
                startActivity(Intent(this@CleanupSettings, PromptsActivity::class.java))
            }
        }

        card("Clés enregistrées", R.drawable.ic_lock).anchor("keys").apply {
            note("Une clé par fournisseur, chiffrée sur le téléphone : changer de fournisseur ne fait pas perdre la clé du précédent, et une clé n'est jamais envoyée à un autre fournisseur. Les clés ne sont jamais affichées.")
            keysList = LinearLayout(this@CleanupSettings).apply { orientation = LinearLayout.VERTICAL }
            addView(keysList)
            keysEmpty = note("Aucune clé enregistrée : saisissez-la sous le fournisseur, puis « Enregistrer le fournisseur ».")
            keysMessage = notice()
        }
        refresh()
    }

    override fun onResume() {
        super.onResume()
        // Retour des prompts : nom du prompt actif à jour.
        if (::promptsStatus.isInitialized) refreshPrompts()
    }

    override fun canLeave(): Boolean {
        val changed = baseUrl.text.toString().trim() != savedUrl || llmModel.text.toString().trim() != savedModel ||
            apiKey.text.isNotBlank()
        if (changed) askSave { saveForm() }
        return !changed
    }

    /** Réglages relus et affichés. */
    private fun refresh() {
        val view = Config.view(store) ?: return say("Réglages illisibles.")
        val llm = view.getJSONObject("llm")
        llmEnabled.quietly(llm.getBoolean("enabled"))
        val list = view.getJSONArray("providers")
        providers = List(list.length()) { list.getJSONObject(it) }
        syncing = true
        provider.adapter = ArrayAdapter(this, android.R.layout.simple_spinner_dropdown_item, providers.map { it.getString("label") })
        shownProvider = -1
        savedUrl = llm.getString("base_url")
        savedModel = llm.getString("model")
        baseUrl.setText(savedUrl)
        llmModel.setText(savedModel)
        syncing = false
        apiKey.setText("")
        refreshKeyTarget(selectProvider = true)
        // Serveur à soi : l'adresse fait partie des réglages de base.
        if (providers.getOrNull(shownProvider)?.optString("id") in setOf("custom", "ollama")) advanced.open()
        refreshKeys()
        refreshPrompts()
    }

    private fun refreshPrompts() {
        val view = Config.view(store) ?: return
        val prompt = { name: String -> if (name.isEmpty()) "intégré" else "« $name »" }
        promptsStatus.summary(
            "Consignes données au LLM. Nettoyage : ${prompt(view.getString("cleanup_prompt"))} · " +
                "Command Mode : ${prompt(view.getString("command_prompt"))}",
        )
    }

    /** Interrupteur : enregistré aussitôt ; sans fournisseur valable, il revient en arrière. */
    private fun switchCleanup(on: Boolean) {
        val error = Config.save(store, JSONObject().put("llm_enabled", on))
        if (error != null) {
            llmEnabled.quietly(!on)
            say("Nettoyage non activé : $error.\nChoisissez d'abord un fournisseur, saisissez sa clé, puis « Enregistrer le fournisseur ».")
            return
        }
        saved(if (on) "✓ Nettoyage activé : dès la prochaine dictée" else "✓ Nettoyage désactivé : le texte brut sera inséré")
    }

    /** Champ de clé : masqué, jamais prérempli, sans suggestions du clavier. */
    private fun LinearLayout.keyField(hint: String): EditText = field(hint).apply {
        inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD or
            InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
        // Android passe les champs de mot de passe en police à chasse fixe.
        typeface = Typeface.DEFAULT
        importantForAutofill = View.IMPORTANT_FOR_AUTOFILL_NO
    }

    /** Préréglage choisi dans le menu : il remplit l'adresse et le modèle. */
    private fun chooseProvider(position: Int) {
        val chosen = providers.getOrNull(position) ?: return
        val current = JSONObject(Native.keyTarget(baseUrl.text.toString())).optJSONObject("ok")
        providerNote.say(chosen.optString("note"))
        if (current?.optString("provider") == chosen.getString("id")) return
        syncing = true
        baseUrl.setText(chosen.optString("base_url"))
        baseUrl.hint = chosen.optString("hint").ifEmpty { "Adresse de l'API" }
        llmModel.setText(chosen.optString("model"))
        llmModel.hint = if (chosen.optString("model").isEmpty()) "Nom exact du modèle" else "Modèle"
        syncing = false
        refreshKeyTarget(selectProvider = false)
        // Rien de prérempli : l'adresse ou le modèle est à saisir, partie avancée ouverte.
        if (chosen.optString("base_url").isEmpty() || chosen.optString("model").isEmpty()) {
            advanced.open()
            (if (chosen.optString("base_url").isEmpty()) baseUrl else llmModel).requestFocus()
        }
    }

    /**
     * État de la clé pour l'adresse saisie (rangée par hôte, comme sur PC) ;
     * `selectProvider` : menu calé sur le fournisseur reconnu.
     */
    private fun refreshKeyTarget(selectProvider: Boolean) {
        val target = JSONObject(Native.keyTarget(baseUrl.text.toString())).optJSONObject("ok") ?: return
        val host = target.getString("host")
        if (selectProvider) {
            var index = providers.indexOfFirst { it.getString("id") == target.getString("provider") }
            // Adresse d'Ollama en cours de saisie (http://192.168…) : le choix reste.
            val current = providers.getOrNull(shownProvider)
            val typing = !baseUrl.text.trim().startsWith("https://")
            if (target.getString("provider") == "custom" && current?.optString("id") == "ollama" && typing) {
                index = shownProvider
            }
            if (index >= 0 && index != shownProvider) {
                shownProvider = index
                provider.setSelection(index, false)
            }
            providerNote.say(providers.getOrNull(index)?.optString("note").orEmpty())
        }
        apiKey.hint = if (host.isEmpty()) "Clé API (jamais affichée)" else "Clé API pour $host (jamais affichée)"
        keyStatus.text = when {
            host.isEmpty() -> "Saisissez l'adresse de l'API (avancé) : la clé est rangée par fournisseur."
            target.getBoolean("cleartext") -> "Adresse en http:// : une clé ne serait jamais envoyée en clair hors du téléphone."
            store.hasKey(host) -> "✓ Clé enregistrée pour $host. Saisissez-en une autre pour la remplacer."
            target.getBoolean("optional") -> "Serveur du réseau local : clé facultative."
            else -> "Aucune clé pour $host : saisissez-la ci-dessus."
        }
    }

    /** Carte « Clés enregistrées » : une ligne par fournisseur ayant une clé. */
    private fun refreshKeys() {
        keysList.removeAllViews()
        val reply = JSONObject(Native.apiKeys(store.config, JSONArray(store.keyHosts()).toString()))
        val entries = reply.optJSONArray("ok") ?: run {
            keysMessage.say(reply.optString("error"))
            return
        }
        for (i in 0 until entries.length()) keyItem(entries.getJSONObject(i))
        keysEmpty.visibility = if (entries.length() == 0) View.VISIBLE else View.GONE
    }

    private fun keyItem(entry: JSONObject) {
        val host = entry.getString("host")
        val usedBy = entry.getJSONArray("used_by").let { a -> List(a.length()) { a.getString(it) } }
        val item = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        keysList.addView(item)
        item.divider(6)
        val name = entry.optString("provider")
        item.text(if (name.isEmpty()) host else "$name · $host", 15f, bold = true)
        item.text(
            "clé enregistrée · " + if (usedBy.isEmpty()) "inutilisée" else usedBy.joinToString(", "),
            13f,
            R.color.muted,
        )
        // Nouvelle clé : champ masqué, vide, jamais prérempli.
        val form = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            visibility = View.GONE
        }
        lateinit var input: EditText
        lateinit var remove: Button
        var confirming = false
        val unconfirm = Runnable {
            confirming = false
            remove.text = "Supprimer"
        }
        val close = {
            input.setText("")
            form.visibility = View.GONE
        }
        lateinit var result: TextView
        lateinit var test: Button
        item.row {
            // Essai de la clé enregistrée, avec l'adresse et le modèle qui l'utilisent.
            test = secondary("Tester") {
                val key = store.key(host)
                if (key == null) {
                    result.say("Clé illisible : remplacez-la.")
                    return@secondary
                }
                runTest(entry.getString("base_url"), entry.optString("model"), key, test, result, models = null)
            }
            secondary("Remplacer") {
                input.setText("")
                form.visibility = if (form.visibility == View.VISIBLE) View.GONE else View.VISIBLE
                if (form.visibility == View.VISIBLE) input.requestFocus()
            }
            // Suppression en deux appuis : « Supprimer » puis « Confirmer ».
            remove = secondary("Supprimer") {
                if (!confirming) {
                    confirming = true
                    remove.text = "Confirmer"
                    main.postDelayed(unconfirm, CONFIRM_MS)
                    return@secondary
                }
                main.removeCallbacks(unconfirm)
                store.deleteKey(host)
                result.say("")
                keysMessage.say("Clé supprimée pour $host.")
                refreshKeyTarget(selectProvider = false)
                refreshKeys()
            }.apply { setTextColor(getColor(R.color.signal)) }
        }
        result = item.notice()
        item.addView(form)
        input = form.keyField("Nouvelle clé pour $host")
        form.row {
            primary("Enregistrer") {
                val key = input.text.toString()
                if (key.isBlank()) {
                    keysMessage.say("Clé vide : rien n'a changé.")
                    return@primary
                }
                store.saveKey(host, key)
                close()
                keysMessage.say("✓ Clé remplacée pour $host.")
                refreshKeyTarget(selectProvider = false)
                refreshKeys()
            }
            secondary("Annuler") { close() }
        }
    }

    /** « Tester » : adresse et modèle saisis, clé saisie sinon celle enregistrée. */
    private fun testForm() {
        val url = baseUrl.text.toString()
        val host = JSONObject(Native.keyTarget(url)).optJSONObject("ok")?.optString("host").orEmpty()
        val key = apiKey.text.toString().trim().ifEmpty { if (host.isEmpty()) "" else store.key(host).orEmpty() }
        runTest(url, llmModel.text.toString(), key, testButton, testResult, testModels)
    }

    /**
     * Test en tâche de fond (réseau), sans rien enregistrer ; la clé ne sort
     * jamais du moteur, le compte rendu ne la contient pas. `models` : liste
     * des modèles du serveur, un appui pour choisir (formulaire seulement).
     */
    private fun runTest(url: String, model: String, key: String, button: Button, result: TextView, models: FlowLayout?) {
        button.isEnabled = false
        result.say("Test en cours…")
        models?.visibility = View.GONE
        val request = JSONObject().put("base_url", url).put("model", model).put("key", key).toString()
        val config = store.config
        tests.execute {
            val reply = runCatching { JSONObject(Native.testLlm(config, request)) }
                .getOrElse { JSONObject().put("error", it.message ?: it.toString()) }
            main.post {
                if (isDestroyed) return@post
                button.isEnabled = true
                val report = reply.optJSONObject("ok")
                if (report == null) {
                    result.say("✗ ${reply.optString("error")}")
                    return@post
                }
                result.say(describe(report))
                if (models != null) showModels(report, models)
            }
        }
    }

    private fun describe(report: JSONObject): String {
        val seconds = String.format(java.util.Locale.FRANCE, "%.1f", report.optLong("duration_ms") / 1000.0)
        val host = report.optString("host")
        val keyNote = "clé : ${report.optString("key")}"
        val lines = mutableListOf<String>()
        val output = report.optString("output").takeIf { !report.isNull("output") }
        when {
            report.optBoolean("ok") && output != null -> {
                lines += "✓ $host répond en $seconds s ($keyNote)."
                lines += "Exemple : « ${report.optString("input")} »\n→ « $output »"
                report.optJSONObject("usage")?.let { usage ->
                    val cost = if (report.isNull("cost")) "" else
                        " · coût ≈ ${String.format(java.util.Locale.FRANCE, "%.5f", report.optDouble("cost"))} ${report.optString("currency")}"
                    lines += "Jetons : ${usage.optLong("input_tokens")} envoyés, ${usage.optLong("output_tokens")} reçus$cost"
                }
            }
            report.optBoolean("ok") ->
                lines += "✓ Clé acceptée par $host : ${report.optJSONArray("models")?.length() ?: 0} modèle(s) disponible(s)."
            else -> {
                val error = report.optString("error").takeIf { !report.isNull("error") }
                    ?: report.optString("models_error")
                lines += "✗ Échec avec $host ($keyNote) :\n$error"
            }
        }
        val hints = report.optJSONArray("hints")
        if (hints != null && hints.length() > 0) {
            lines += "Pistes :\n" + (0 until hints.length()).joinToString("\n") { "• ${hints.getString(it)}" }
        }
        if (report.optBoolean("ok") && output != null && !report.isNull("models_error")) {
            lines += "Liste des modèles indisponible : ${report.optString("models_error")}"
        }
        return lines.joinToString("\n\n")
    }

    /** Modèles annoncés par le serveur : un appui remplit le champ Modèle. */
    private fun showModels(report: JSONObject, flow: FlowLayout) {
        flow.removeAllViews()
        val list = report.optJSONArray("models") ?: return
        if (list.length() == 0) return
        val names = List(list.length()) { list.getString(it) }
        val shown = names.take(MAX_MODELS)
        flow.addView(TextView(this).apply {
            text = "Modèles du serveur (toucher pour choisir)" +
                if (names.size > shown.size) ", ${shown.size} sur ${names.size} :" else " :"
            setTextSize(TypedValue.COMPLEX_UNIT_SP, 14f)
            setTextColor(getColor(R.color.muted))
        })
        for (name in shown) {
            flow.addView(TextView(this).apply {
                text = name
                setTextSize(TypedValue.COMPLEX_UNIT_SP, 14f)
                setTextColor(getColor(R.color.fg))
                val selected = name == llmModel.text.toString().trim()
                background = pressable(
                    rounded(getColor(if (selected) R.color.card else R.color.bg), 999f, getColor(if (selected) R.color.signal else R.color.border_strong)),
                    999f,
                )
                setPadding(dp(12), dp(6), dp(12), dp(6))
                setOnClickListener {
                    llmModel.setText(name)
                    advanced.open()
                    showModels(report, flow)
                    formMessage.say("Modèle « $name » choisi : « Enregistrer le fournisseur » pour l'utiliser.")
                }
            })
        }
        flow.visibility = View.VISIBLE
    }

    /** « Enregistrer le fournisseur » : adresse, modèle, puis clé rangée pour cette adresse. */
    private fun saveForm(): Boolean {
        val error = Config.save(
            store,
            JSONObject().put("base_url", baseUrl.text.toString()).put("llm_model", llmModel.text.toString()),
        )
        if (error != null) {
            formMessage.say("✗ Non enregistré : $error")
            formMessage.reveal()
            return false
        }
        val llm = Config.view(store)?.getJSONObject("llm")
        val target = JSONObject(Native.keyTarget(llm?.optString("base_url").orEmpty())).optJSONObject("ok")
        val newHost = target?.optString("host").orEmpty()
        val key = apiKey.text.toString()
        val keyNote = when {
            key.isBlank() -> ""
            newHost.isEmpty() -> "\nClé non enregistrée : adresse de l'API invalide."
            target?.optBoolean("cleartext") == true ->
                "\nClé non enregistrée : adresse en http://, elle ne serait jamais envoyée en clair hors du téléphone."
            else -> {
                store.saveKey(newHost, key)
                "\n✓ Clé enregistrée pour $newHost."
            }
        }
        refresh()
        val enabled = llm?.optBoolean("enabled") == true
        formMessage.say(
            "✓ Fournisseur enregistré" +
                (if (enabled) " : utilisé dès la prochaine dictée." else ". Le nettoyage est désactivé : activez-le en haut de la page.") +
                keyNote,
        )
        saved("✓ Fournisseur enregistré")
        return true
    }

    override fun onDestroy() {
        tests.shutdown()
        super.onDestroy()
    }

    private companion object {
        /** Délai pour confirmer la suppression d'une clé. */
        const val CONFIRM_MS = 4000L

        /** Modèles affichés après un test (OpenRouter en annonce des centaines). */
        const val MAX_MODELS = 40
    }
}
