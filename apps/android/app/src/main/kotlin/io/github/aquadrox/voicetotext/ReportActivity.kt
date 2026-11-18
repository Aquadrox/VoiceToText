package io.github.aquadrox.voicetotext

import android.annotation.SuppressLint
import android.app.Activity
import android.content.Context
import android.content.Intent
import android.content.res.ColorStateList
import android.content.res.Configuration
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioTrack
import android.net.Uri
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.text.TextUtils
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.webkit.JavascriptInterface
import android.webkit.WebResourceRequest
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
import org.json.JSONObject
import java.text.DateFormat
import java.util.Date
import java.util.concurrent.Executors

/**
 * Rapport complet d'une réunion, plein écran : le document HTML rédigé par
 * le moteur (sommaire, recherche, filtres, transcription par chapitre),
 * affiché dans une WebView sans accès au réseau ni aux fichiers. L'objet
 * `VTT` relie la page à l'application : ▶ (réécoute) et cases du plan
 * d'action (enregistrées dans la réunion).
 */
class ReportActivity : Activity() {
    private val store by lazy { Store(this) }
    private val main = Handler(Looper.getMainLooper())
    private val worker = Executors.newSingleThreadExecutor()
    private var meetingId = ""
    private var title = ""
    private var date = ""
    private lateinit var web: WebView
    private var player: AudioTrack? = null

    @SuppressLint("SetJavaScriptEnabled", "AddJavascriptInterface")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        meetingId = intent.getStringExtra(MeetingService.EXTRA_ID).orEmpty()
        val detail = JSONObject(Native.meeting(store.meetingsDir, meetingId)).optJSONObject("ok")
        if (detail == null) {
            finish()
            return
        }
        title = detail.optString("title")
        date = Reports.date(detail)

        val bar = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setBackgroundColor(getColor(R.color.card))
            setPadding(dp(4), dp(4), dp(4), dp(4))
        }
        bar.addView(barButton(R.drawable.ic_arrow_left, "Retour") { finish() }, LinearLayout.LayoutParams(dp(48), dp(48)))
        bar.addView(
            TextView(this).apply {
                text = title
                setTextColor(getColor(R.color.fg))
                setTextSize(TypedValue.COMPLEX_UNIT_SP, 17f)
                typeface = android.graphics.Typeface.create(android.graphics.Typeface.DEFAULT, android.graphics.Typeface.BOLD)
                maxLines = 1
                ellipsize = TextUtils.TruncateAt.END
                setPadding(dp(4), 0, dp(4), 0)
            },
            LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f),
        )
        bar.addView(barButton(R.drawable.ic_upload, "Enregistrer en HTML") { Reports.save(this, title) }, LinearLayout.LayoutParams(dp(48), dp(48)))
        bar.addView(barButton(R.drawable.ic_share, "Partager le rapport (texte)") { share() }, LinearLayout.LayoutParams(dp(48), dp(48)))

        web = WebView(this).apply {
            setBackgroundColor(getColor(R.color.bg))
            settings.javaScriptEnabled = true
            // Document autonome : ni réseau, ni fichiers, ni stockage.
            settings.blockNetworkLoads = true
            settings.allowFileAccess = false
            settings.allowContentAccess = false
            settings.domStorageEnabled = false
            settings.setGeolocationEnabled(false)
            settings.builtInZoomControls = false
            // Taille du texte choisie dans les réglages du téléphone.
            settings.textZoom = (resources.configuration.fontScale * 100).toInt()
            webViewClient = object : WebViewClient() {
                // Aucune navigation hors du document (ancres gérées par la page).
                override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean = true
            }
            addJavascriptInterface(Bridge(), "VTT")
        }

        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            fitsSystemWindows = true
            setBackgroundColor(getColor(R.color.card))
            addView(bar, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT))
            addView(View(this@ReportActivity).apply { setBackgroundColor(getColor(R.color.border)) }, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, dp(1)))
            addView(web, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f))
        }
        setContentView(root)
        load()
    }

    private fun barButton(icon: Int, description: String, action: () -> Unit): View =
        iconButton(icon, description, action).apply {
            imageTintList = ColorStateList.valueOf(getColor(R.color.fg))
        }

    private fun load() {
        val night = resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK == Configuration.UI_MODE_NIGHT_YES
        val reply = JSONObject(Native.meetingReportHtml(store.meetingsDir, meetingId, date, if (night) "dark" else "light"))
        val html = reply.optString("ok")
        if (html.isEmpty()) {
            Toast.makeText(this, reply.optString("error", "Rapport indisponible"), Toast.LENGTH_LONG).show()
            finish()
            return
        }
        web.loadDataWithBaseURL(null, html, "text/html", "utf-8", null)
    }

    override fun onPause() {
        stopPlayer()
        super.onPause()
    }

    override fun onDestroy() {
        worker.shutdown()
        if (::web.isInitialized) {
            web.removeJavascriptInterface("VTT")
            web.destroy()
        }
        super.onDestroy()
    }

    @Deprecated("Activity.onActivityResult")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        Reports.onSaved(this, requestCode, resultCode, data, meetingId, date)
    }

    private fun share() {
        val text = JSONObject(Native.meetingExport(store.meetingsDir, meetingId, "report", date)).optString("ok")
        if (text.isNotEmpty()) Meetings.share(this, "Rapport : $title", text)
    }

    /** Appelé par la page (fil de la WebView) : tout repasse sur le fil principal. */
    private inner class Bridge {
        @JavascriptInterface
        fun play(start: Long, end: Long) {
            main.post { play(start, end) }
        }

        @JavascriptInterface
        fun stop() {
            main.post { stopPlayer() }
        }

        @JavascriptInterface
        fun setDone(index: Int, done: Boolean) {
            val root = store.meetingsDir
            worker.execute {
                val error = JSONObject(Native.meetingActionDone(root, meetingId, index, done)).optString("error")
                if (error.isNotEmpty()) main.post { Toast.makeText(this@ReportActivity, error, Toast.LENGTH_SHORT).show() }
            }
        }
    }

    // ---- Réécoute ----

    private fun play(start: Long, end: Long) {
        stopPlayer()
        val pcm = Native.meetingClip(store.meetingsDir, meetingId, start, end)
        if (pcm == null || pcm.isEmpty()) {
            Toast.makeText(this, "Audio indisponible", Toast.LENGTH_SHORT).show()
            return
        }
        val track = AudioTrack.Builder()
            .setAudioAttributes(AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA).setContentType(AudioAttributes.CONTENT_TYPE_SPEECH).build())
            .setAudioFormat(AudioFormat.Builder().setSampleRate(16_000).setChannelMask(AudioFormat.CHANNEL_OUT_MONO).setEncoding(AudioFormat.ENCODING_PCM_16BIT).build())
            .setTransferMode(AudioTrack.MODE_STATIC)
            .setBufferSizeInBytes(pcm.size * 2)
            .build()
        track.write(pcm, 0, pcm.size)
        track.notificationMarkerPosition = pcm.size
        track.setPlaybackPositionUpdateListener(object : AudioTrack.OnPlaybackPositionUpdateListener {
            override fun onMarkerReached(t: AudioTrack?) = stopPlayer()
            override fun onPeriodicNotification(t: AudioTrack?) {}
        })
        track.play()
        player = track
        web.evaluateJavascript("window.vttPlaying && window.vttPlaying($start)", null)
    }

    private fun stopPlayer() {
        val track = player ?: return
        runCatching { track.stop() }
        track.release()
        player = null
        if (::web.isInitialized) web.evaluateJavascript("window.vttPlaying && window.vttPlaying(-1)", null)
    }

    companion object {
        fun intent(context: Context, meetingId: String): Intent =
            Intent(context, ReportActivity::class.java).putExtra(MeetingService.EXTRA_ID, meetingId)
    }
}

/** Enregistrement du rapport en HTML (document autonome), partagé par les écrans. */
object Reports {
    const val SAVE_HTML = 4201

    /** Date lisible de la réunion, en tête du rapport. */
    fun date(detail: JSONObject): String =
        DateFormat.getDateTimeInstance(DateFormat.LONG, DateFormat.SHORT).format(Date(detail.optLong("started_at_ms")))

    /** Choix de l'emplacement (Téléchargements, Drive…) : réponse dans [onSaved]. */
    fun save(activity: Activity, title: String) {
        val name = title.replace(Regex("[\\\\/:*?\"<>|]"), " ").trim().ifEmpty { "Réunion" }
        val intent = Intent(Intent.ACTION_CREATE_DOCUMENT)
            .addCategory(Intent.CATEGORY_OPENABLE)
            .setType("text/html")
            .putExtra(Intent.EXTRA_TITLE, "$name - rapport.html")
        try {
            @Suppress("DEPRECATION")
            activity.startActivityForResult(intent, SAVE_HTML)
        } catch (e: android.content.ActivityNotFoundException) {
            Toast.makeText(activity, "Aucune application pour enregistrer un fichier", Toast.LENGTH_LONG).show()
        }
    }

    fun onSaved(activity: Activity, requestCode: Int, resultCode: Int, data: Intent?, meetingId: String, date: String) {
        if (requestCode != SAVE_HTML || resultCode != Activity.RESULT_OK) return
        val uri: Uri = data?.data ?: return
        val reply = JSONObject(Native.meetingExport(Store(activity).meetingsDir, meetingId, "report_html", date))
        val html = reply.optString("ok")
        val message = if (html.isEmpty()) {
            reply.optString("error", "Rapport indisponible")
        } else {
            runCatching {
                activity.contentResolver.openOutputStream(uri, "wt")!!.use { it.write(html.toByteArray(Charsets.UTF_8)) }
                "Rapport enregistré (HTML) : il s'ouvre dans n'importe quel navigateur."
            }.getOrElse { "Enregistrement impossible : ${it.message}" }
        }
        Toast.makeText(activity, message, Toast.LENGTH_LONG).show()
    }
}
