package io.github.aquadrox.voicetotext

import android.app.Activity
import android.content.Context
import android.content.res.ColorStateList
import android.graphics.Typeface
import android.graphics.drawable.Drawable
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.RippleDrawable
import android.graphics.drawable.StateListDrawable
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.Button
import android.widget.EditText
import android.widget.ImageButton
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.Switch
import android.widget.TextView

/*
 * Éléments d'interface communs aux écrans, sans bibliothèque : en-tête, cartes,
 * boutons, champs. Couleurs : res/values/colors.xml (clair) et
 * res/values-night/colors.xml (sombre), tirées du logo comme sur PC.
 */

private const val MATCH = ViewGroup.LayoutParams.MATCH_PARENT
private const val WRAP = ViewGroup.LayoutParams.WRAP_CONTENT

fun Context.dp(value: Int) = (value * resources.displayMetrics.density).toInt()

private fun Context.dpf(value: Float) = value * resources.displayMetrics.density

/** Fond arrondi, avec bordure facultative. */
fun Context.rounded(fill: Int, radius: Float, stroke: Int? = null, strokeWidth: Int = 1): GradientDrawable =
    GradientDrawable().apply {
        setColor(fill)
        cornerRadius = dpf(radius)
        if (stroke != null) setStroke(dp(strokeWidth), stroke)
    }

/** Effet d'appui (ondulation) sur un fond, limité à ses coins arrondis. */
fun Context.pressable(background: Drawable?, radius: Float, ripple: Int = getColor(R.color.ripple)): RippleDrawable {
    val mask = GradientDrawable().apply {
        setColor(0xFFFFFFFF.toInt())
        cornerRadius = dpf(radius)
    }
    return RippleDrawable(ColorStateList.valueOf(ripple), background, mask)
}

/**
 * Écran complet : colonne défilante dans les marges des barres d'Android
 * (Android 15 et plus dessinent l'application sous ces barres), avec une
 * barre d'actions facultative fixée en bas.
 */
fun Activity.screen(bar: (LinearLayout.() -> Unit)? = null, content: LinearLayout.() -> Unit) {
    val column = LinearLayout(this).apply {
        orientation = LinearLayout.VERTICAL
        setPadding(dp(16), dp(12), dp(16), dp(24))
        content()
    }
    val root = LinearLayout(this).apply {
        orientation = LinearLayout.VERTICAL
        fitsSystemWindows = true
        setBackgroundColor(getColor(R.color.bg))
        addView(ScrollView(this@screen).apply { addView(column) }, LinearLayout.LayoutParams(MATCH, 0, 1f))
        if (bar != null) {
            addView(View(this@screen).apply { setBackgroundColor(getColor(R.color.border)) }, LinearLayout.LayoutParams(MATCH, dp(1)))
            addView(
                LinearLayout(this@screen).apply {
                    orientation = LinearLayout.HORIZONTAL
                    setBackgroundColor(getColor(R.color.card))
                    setPadding(dp(16), dp(4), dp(16), dp(12))
                    bar()
                },
                LinearLayout.LayoutParams(MATCH, WRAP),
            )
        }
    }
    setContentView(root)
}

/** Texte simple ; taille en sp. */
fun LinearLayout.text(value: CharSequence, size: Float = 15f, color: Int = R.color.fg, bold: Boolean = false): TextView =
    TextView(context).apply {
        text = value
        setTextSize(TypedValue.COMPLEX_UNIT_SP, size)
        setTextColor(context.getColor(color))
        if (bold) typeface = Typeface.create(Typeface.DEFAULT, Typeface.BOLD)
        setLineSpacing(0f, 1.15f)
    }.also { addView(it, LinearLayout.LayoutParams(if (orientation == LinearLayout.HORIZONTAL) WRAP else MATCH, WRAP)) }

/** Explication en gris. */
fun LinearLayout.note(value: String): TextView = text(value, 14f, R.color.muted).apply {
    setPadding(0, context.dp(4), 0, context.dp(4))
}

/** Message d'état (enregistré, erreur…) : encadré, caché tant qu'il est vide. */
fun LinearLayout.notice(): TextView = text("", 14f).apply {
    background = context.rounded(context.getColor(R.color.card), 12f, context.getColor(R.color.border_strong))
    setPadding(context.dp(14), context.dp(10), context.dp(14), context.dp(10))
    (layoutParams as LinearLayout.LayoutParams).topMargin = context.dp(8)
    visibility = View.GONE
}

/** Affiche un message d'état ; vide : caché. */
fun TextView.say(value: String) {
    text = value
    visibility = if (value.isEmpty()) View.GONE else View.VISIBLE
}

/** En-tête : logo (accueil) ou retour, titre, sous-titre. Renvoie le sous-titre. */
fun LinearLayout.header(title: String, subtitle: String = "", onBack: (() -> Unit)? = null): TextView {
    val ctx = context
    val row = LinearLayout(ctx).apply {
        orientation = LinearLayout.HORIZONTAL
        gravity = Gravity.CENTER_VERTICAL
        setPadding(0, ctx.dp(8), 0, ctx.dp(8))
    }
    val lead: View = if (onBack != null) {
        ImageButton(ctx).apply {
            setImageResource(R.drawable.ic_arrow_left)
            imageTintList = ColorStateList.valueOf(ctx.getColor(R.color.fg))
            background = ctx.pressable(null, 20f)
            setPadding(ctx.dp(8), ctx.dp(8), ctx.dp(8), ctx.dp(8))
            contentDescription = "Retour"
            setOnClickListener { onBack() }
        }
    } else {
        ImageView(ctx).apply {
            setImageResource(R.drawable.ic_logo)
            importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
        }
    }
    row.addView(lead, LinearLayout.LayoutParams(ctx.dp(40), ctx.dp(40)).apply { marginEnd = ctx.dp(12) })
    val texts = LinearLayout(ctx).apply { orientation = LinearLayout.VERTICAL }
    texts.text(title, 22f, bold = true)
    val sub = texts.text(subtitle, 13f, R.color.muted)
    if (subtitle.isEmpty()) sub.visibility = View.GONE
    row.addView(texts, LinearLayout.LayoutParams(0, WRAP, 1f))
    addView(row, LinearLayout.LayoutParams(MATCH, WRAP))
    return sub
}

/** Carte : bloc à bord fin, titre (avec icône) puis contenu en colonne. */
fun LinearLayout.card(title: String = "", icon: Int = 0): LinearLayout {
    val ctx = context
    val card = LinearLayout(ctx).apply {
        orientation = LinearLayout.VERTICAL
        background = ctx.rounded(ctx.getColor(R.color.card), 14f, ctx.getColor(R.color.border))
        setPadding(ctx.dp(16), ctx.dp(14), ctx.dp(16), ctx.dp(16))
    }
    if (title.isNotEmpty()) card.cardTitle(title, icon)
    addView(card, LinearLayout.LayoutParams(MATCH, WRAP).apply { topMargin = ctx.dp(12) })
    return card
}

private fun LinearLayout.cardTitle(title: String, icon: Int) {
    val ctx = context
    val row = LinearLayout(ctx).apply {
        orientation = LinearLayout.HORIZONTAL
        gravity = Gravity.CENTER_VERTICAL
        setPadding(0, 0, 0, ctx.dp(4))
    }
    if (icon != 0) {
        row.addView(
            ImageView(ctx).apply {
                setImageResource(icon)
                imageTintList = ColorStateList.valueOf(ctx.getColor(R.color.signal))
                importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
            },
            LinearLayout.LayoutParams(ctx.dp(20), ctx.dp(20)).apply { marginEnd = ctx.dp(10) },
        )
    }
    row.text(title, 17f, bold = true)
    addView(row, LinearLayout.LayoutParams(MATCH, WRAP))
}

/**
 * Rangée horizontale (boutons côte à côte, champ et bouton…), alignée en
 * haut : champs et boutons ont la même hauteur et la même marge. (Centrer
 * ajouterait la marge après le centrage et rognerait le bas.)
 */
fun LinearLayout.row(content: LinearLayout.() -> Unit): LinearLayout = LinearLayout(context).apply {
    orientation = LinearLayout.HORIZONTAL
    content()
}.also { addView(it, LinearLayout.LayoutParams(MATCH, WRAP)) }

/** Bouton principal : plein, couleur d'action (encre ; papier en mode sombre). */
fun LinearLayout.primary(label: String, action: () -> Unit): Button = styledButton(label, true, action)

/** Bouton secondaire : contour fin. */
fun LinearLayout.secondary(label: String, action: () -> Unit): Button = styledButton(label, false, action)

private fun LinearLayout.styledButton(label: String, filled: Boolean, action: () -> Unit): Button {
    val ctx = context
    val button = object : Button(ctx) {
        // Désactivé : estompé (le fond dessiné ne change pas de lui-même).
        override fun setEnabled(enabled: Boolean) {
            super.setEnabled(enabled)
            alpha = if (enabled) 1f else 0.45f
        }
    }
    button.apply {
        text = label
        isAllCaps = false
        setTextSize(TypedValue.COMPLEX_UNIT_SP, 15f)
        typeface = Typeface.create(Typeface.DEFAULT, if (filled) Typeface.BOLD else Typeface.NORMAL)
        setTextColor(ctx.getColor(if (filled) R.color.accent_fg else R.color.fg))
        val fill = ctx.getColor(if (filled) R.color.accent else R.color.card)
        val stroke = if (filled) null else ctx.getColor(R.color.border_strong)
        background = ctx.pressable(ctx.rounded(fill, 10f, stroke), 10f)
        stateListAnimator = null
        minHeight = ctx.dp(48)
        minimumHeight = ctx.dp(48)
        setPadding(ctx.dp(14), 0, ctx.dp(14), 0)
        setOnClickListener { action() }
    }
    val params = if (orientation == LinearLayout.HORIZONTAL) {
        LinearLayout.LayoutParams(0, WRAP, 1f).apply {
            if (childCount > 0) marginStart = ctx.dp(8)
            topMargin = ctx.dp(8)
        }
    } else {
        LinearLayout.LayoutParams(MATCH, WRAP).apply { topMargin = ctx.dp(8) }
    }
    addView(button, params)
    return button
}

/** Champ de saisie : fond arrondi, bord vermillon quand il a le focus. */
fun LinearLayout.field(hint: String, lines: Int = 1): EditText {
    val ctx = context
    val bg = ctx.getColor(R.color.bg)
    return EditText(ctx).apply {
        this.hint = hint
        setTextSize(TypedValue.COMPLEX_UNIT_SP, 15f)
        // Même hauteur que les boutons, pour les aligner sur une rangée.
        // Avant `minLines` : Android ne garde que le dernier minimum posé.
        minHeight = ctx.dp(48)
        if (lines == 1) isSingleLine = true else minLines = lines
        setTextColor(ctx.getColor(R.color.fg))
        setHintTextColor(ctx.getColor(R.color.muted))
        background = StateListDrawable().apply {
            addState(intArrayOf(android.R.attr.state_focused), ctx.rounded(bg, 10f, ctx.getColor(R.color.signal), 2))
            addState(intArrayOf(), ctx.rounded(bg, 10f, ctx.getColor(R.color.border_strong)))
        }
        setPadding(ctx.dp(12), ctx.dp(10), ctx.dp(12), ctx.dp(10))
    }.also {
        val params = if (orientation == LinearLayout.HORIZONTAL) {
            LinearLayout.LayoutParams(0, WRAP, 1f)
        } else {
            LinearLayout.LayoutParams(MATCH, WRAP)
        }
        addView(it, params.apply { topMargin = ctx.dp(8) })
    }
}

/** Interrupteur (couleurs : thème). `change` : enregistré dès qu'on le touche. */
fun LinearLayout.option(label: String, checked: Boolean = false, change: ((Boolean) -> Unit)? = null): Switch =
    Switch(context).apply {
        text = label
        isChecked = checked
        setTextSize(TypedValue.COMPLEX_UNIT_SP, 15f)
        setTextColor(context.getColor(R.color.fg))
        switchPadding = context.dp(12)
        setPadding(0, context.dp(10), 0, context.dp(10))
        if (change != null) setOnCheckedChangeListener { _, value -> change(value) }
    }.also { addView(it, LinearLayout.LayoutParams(MATCH, WRAP)) }

/** Trait de séparation ; `margin` en dp au-dessus et au-dessous. */
fun LinearLayout.divider(margin: Int = 8) {
    addView(
        View(context).apply { setBackgroundColor(context.getColor(R.color.border)) },
        LinearLayout.LayoutParams(MATCH, context.dp(1)).apply {
            topMargin = context.dp(margin)
            bottomMargin = context.dp(margin)
        },
    )
}

/** « En savoir plus » : texte replié, déplié d'un appui. */
fun LinearLayout.more(body: String, label: String = "En savoir plus") {
    val ctx = context
    val toggle = text(label, 14f, R.color.fg, bold = true)
    val detail = note(body).apply { visibility = View.GONE }
    toggle.apply {
        setCompoundDrawablesRelativeWithIntrinsicBounds(0, 0, R.drawable.ic_chevron_down, 0)
        compoundDrawableTintList = ColorStateList.valueOf(ctx.getColor(R.color.muted))
        compoundDrawablePadding = ctx.dp(6)
        gravity = Gravity.CENTER_VERTICAL
        setPadding(0, ctx.dp(10), 0, ctx.dp(6))
        background = ctx.pressable(null, 8f)
        setOnClickListener {
            detail.visibility = if (detail.visibility == View.VISIBLE) View.GONE else View.VISIBLE
        }
    }
    (toggle.layoutParams as LinearLayout.LayoutParams).width = WRAP
}

/** Étape de mise en route : coche verte (faite) ou cercle (à faire), titre, détail, actions. */
class Step(private val icon: ImageView, private val detail: TextView, val actions: LinearLayout) {
    fun set(done: Boolean, text: String) {
        icon.setImageResource(if (done) R.drawable.ic_check else R.drawable.ic_circle)
        icon.imageTintList = ColorStateList.valueOf(icon.context.getColor(if (done) R.color.ok else R.color.muted))
        icon.contentDescription = if (done) "Fait" else "À faire"
        detail.text = text
    }
}

fun LinearLayout.step(title: String): Step {
    val ctx = context
    val line = LinearLayout(ctx).apply {
        orientation = LinearLayout.HORIZONTAL
        setPadding(0, ctx.dp(10), 0, ctx.dp(4))
    }
    val icon = ImageView(ctx)
    line.addView(icon, LinearLayout.LayoutParams(ctx.dp(22), ctx.dp(22)).apply {
        marginEnd = ctx.dp(12)
        topMargin = ctx.dp(1)
    })
    val texts = LinearLayout(ctx).apply { orientation = LinearLayout.VERTICAL }
    texts.text(title, 15f, bold = true)
    val detail = texts.text("", 14f, R.color.muted)
    val actions = LinearLayout(ctx).apply { orientation = LinearLayout.HORIZONTAL }
    texts.addView(actions, LinearLayout.LayoutParams(MATCH, WRAP))
    line.addView(texts, LinearLayout.LayoutParams(0, WRAP, 1f))
    addView(line, LinearLayout.LayoutParams(MATCH, WRAP))
    return Step(icon, detail, actions).also { it.set(false, "") }
}

/** Éléments à la suite, qui passent à la ligne (pastilles du dictionnaire). */
class FlowLayout(context: Context) : ViewGroup(context) {
    private val gap = context.dp(6)

    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        val width = View.MeasureSpec.getSize(widthMeasureSpec) - paddingLeft - paddingRight
        val childWidth = View.MeasureSpec.makeMeasureSpec(maxOf(0, width), View.MeasureSpec.AT_MOST)
        val childHeight = View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED)
        var x = 0
        var y = 0
        var line = 0
        for (i in 0 until childCount) {
            val child = getChildAt(i)
            if (child.visibility == View.GONE) continue
            child.measure(childWidth, childHeight)
            if (x > 0 && x + child.measuredWidth > width) {
                x = 0
                y += line + gap
                line = 0
            }
            x += child.measuredWidth + gap
            line = maxOf(line, child.measuredHeight)
        }
        val height = y + line + paddingTop + paddingBottom
        setMeasuredDimension(View.MeasureSpec.getSize(widthMeasureSpec), View.resolveSize(height, heightMeasureSpec))
    }

    override fun onLayout(changed: Boolean, l: Int, t: Int, r: Int, b: Int) {
        val width = r - l - paddingLeft - paddingRight
        var x = 0
        var y = 0
        var line = 0
        for (i in 0 until childCount) {
            val child = getChildAt(i)
            if (child.visibility == View.GONE) continue
            if (x > 0 && x + child.measuredWidth > width) {
                x = 0
                y += line + gap
                line = 0
            }
            child.layout(paddingLeft + x, paddingTop + y, paddingLeft + x + child.measuredWidth, paddingTop + y + child.measuredHeight)
            x += child.measuredWidth + gap
            line = maxOf(line, child.measuredHeight)
        }
    }
}

/** Petit bouton icône (retirer, copier…), teinté en gris. */
fun Context.iconButton(icon: Int, description: String, action: () -> Unit): ImageButton = ImageButton(this).apply {
    setImageResource(icon)
    imageTintList = ColorStateList.valueOf(getColor(R.color.muted))
    background = pressable(null, 16f)
    scaleType = ImageView.ScaleType.FIT_CENTER
    setPadding(dp(6), dp(6), dp(6), dp(6))
    contentDescription = description
    setOnClickListener { action() }
}
