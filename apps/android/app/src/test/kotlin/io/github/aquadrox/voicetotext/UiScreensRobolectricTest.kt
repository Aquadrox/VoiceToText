package io.github.aquadrox.voicetotext

import android.app.Activity
import android.view.View
import android.view.ViewGroup
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * Tests de non-régression UI Android (Robolectric) sur les écrans principaux
 * avec le vrai moteur Rust (`libvtt_android.so` chargé par JNI) dans les deux
 * éditions (`clavier` et `bulle`), en mode clair et sombre.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class UiScreensRobolectricTest {

    private fun collectTexts(root: View): List<String> {
        val out = mutableListOf<String>()
        fun walk(v: View) {
            if (v is TextView && v.visibility == View.VISIBLE) {
                val text = v.text?.toString()?.trim().orEmpty()
                if (text.isNotEmpty()) out.add(text)
            }
            if (v is ViewGroup && v.visibility == View.VISIBLE) {
                for (i in 0 until v.childCount) walk(v.getChildAt(i))
            }
        }
        walk(root)
        return out
    }

    private fun <T : View> findFirstOf(root: View, clazz: Class<T>, predicate: (T) -> Boolean): T? {
        if (clazz.isInstance(root)) {
            val typed = clazz.cast(root)!!
            if (predicate(typed)) return typed
        }
        if (root is ViewGroup) {
            for (i in 0 until root.childCount) {
                findFirstOf(root.getChildAt(i), clazz, predicate)?.let { return it }
            }
        }
        return null
    }

    private inline fun <reified T : View> findFirst(root: View, noinline predicate: (T) -> Boolean): T? =
        findFirstOf(root, T::class.java, predicate)

    private fun <T : Activity> launchAndVerify(clazz: Class<T>): T {
        val controller = Robolectric.buildActivity(clazz).create().start().resume()
        val activity = controller.get()
        val root = activity.findViewById<View>(android.R.id.content)
        assertNotNull("${clazz.simpleName} doit monter une vue racine", root)
        assertTrue(
            "${clazz.simpleName} doit afficher du contenu visible",
            collectTexts(root).isNotEmpty(),
        )
        return activity
    }

    @Test
    fun mainActivityRendersCategoriesButtonsAndSettingsSearch() {
        val activity = launchAndVerify(MainActivity::class.java)
        val root = activity.findViewById<View>(android.R.id.content)
        val texts = collectTexts(root)

        assertTrue(texts.any { it.contains("VoiceToText") })
        assertTrue(texts.any { it == "Dictée" })
        assertTrue(texts.any { it == "Nettoyage du texte" })
        assertTrue(texts.any { it == "Dictionnaire et snippets" })
        assertTrue(texts.any { it == "Mises à jour" })
        if (BuildConfig.BUBBLE) {
            assertTrue(texts.any { it == "Bulle de dictée" })
        }

        // Non-régression v0.35.1 : la rangée « Enregistrer » / « Toutes les réunions »
        // ne doit pas aligner ses boutons sur la ligne de base.
        val allMeetingsBtn = findFirst<Button>(root) { it.text?.toString() == "Toutes les réunions" }
        assertNotNull("Bouton « Toutes les réunions » attendu sur l'accueil", allMeetingsBtn)
        val row = allMeetingsBtn?.parent as? LinearLayout
        assertNotNull(row)
        assertFalse(
            "La rangée de boutons ne doit pas être alignée sur la baseline",
            row!!.isBaselineAligned,
        )

        // Recherche dans les réglages : « micro » filtre et affiche le résultat.
        val searchInput = findFirst<EditText>(root) {
            it.hint?.toString()?.contains("Rechercher", ignoreCase = true) == true
        }
        assertNotNull("Champ de recherche des réglages attendu", searchInput)
        searchInput!!.setText("micro")
        val filtered = collectTexts(root)
        assertTrue(
            "La recherche « micro » doit faire apparaître un résultat Micro",
            filtered.any { it.contains("Micro", ignoreCase = true) },
        )
    }

    @Test
    @Config(sdk = [34], qualifiers = "night")
    fun mainActivityRendersInDarkTheme() {
        val activity = launchAndVerify(MainActivity::class.java)
        val texts = collectTexts(activity.findViewById(android.R.id.content))
        assertTrue(texts.any { it.contains("VoiceToText") })
    }

    @Test
    fun meetingsActivityRendersHeaderVoicebookAndActions() {
        val activity = launchAndVerify(MeetingsActivity::class.java)
        val texts = collectTexts(activity.findViewById(android.R.id.content))

        assertTrue(texts.any { it == "Réunions" })
        assertTrue(texts.any { it.contains("Carnet de voix local") })
        assertTrue(texts.any { it == "Enregistrer" })
        assertTrue(texts.any { it == "Importer" })
    }

    @Test
    fun settingsAndSecondaryActivitiesLaunchCleanly() {
        val activities = mutableListOf<Class<out Activity>>(
            DictationSettings::class.java,
            CleanupSettings::class.java,
            HistorySettings::class.java,
            HistoryActivity::class.java,
            VocabularyActivity::class.java,
            PromptsActivity::class.java,
            SyncSettings::class.java,
            UpdatesSettings::class.java,
            HelpSettings::class.java,
        )
        if (BuildConfig.BUBBLE) {
            activities.add(BubbleSettings::class.java)
        }
        for (clazz in activities) {
            val activity = launchAndVerify(clazz)
            activity.finish()
        }
    }

    @Test
    fun voiceKeyboardBuildsInputView() {
        val service = Robolectric.buildService(VoiceKeyboard::class.java).create().get()
        val view = service.onCreateInputView()
        assertNotNull("Le clavier VoiceKeyboard doit produire une vue d'entrée", view)
        assertTrue("La vue du clavier doit être un ViewGroup non vide", view is ViewGroup && view.childCount > 0)
        assertEquals(View.VISIBLE, view.visibility)
    }
}
