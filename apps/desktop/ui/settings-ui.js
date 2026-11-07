// Réglages en sections : une section affichée à la fois, et pour chacune un
// bouton « Appliquer » qui n'applique QUE cette section.
//
// Principe (comme un formulaire .NET avec « valeurs d'origine ») : après
// chaque remplissage depuis la configuration appliquée, la valeur de chaque
// champ est photographiée. Un champ dont la valeur diffère de sa photo est
// « modifié » : il est signalé, la section affiche « Appliquer / Annuler »,
// et un point apparaît dans le menu. Les champs dans [data-untracked] (clé
// API, nom de profil, démarrage avec Windows…) sont appliqués immédiatement
// et ne sont pas suivis.
const SettingsUI = (() => {
  const DEFAULT_PANEL = "raccourcis";
  const LAST_PANEL_KEY = "vtt-settings-panel";
  /** nom → { element, snapshot: Map<élément, valeur>, apply, revert } */
  const panels = {};

  for (const element of document.querySelectorAll(".panel[data-panel]")) {
    panels[element.dataset.panel] = { element, snapshot: null, apply: null, revert: null };
  }

  const valueOf = (control) =>
    control.type === "checkbox" || control.type === "radio" ? control.checked : control.value;

  function setValue(control, value) {
    if (control.type === "checkbox" || control.type === "radio") control.checked = value;
    else control.value = value;
  }

  /** Champs suivis d'une section (relus à chaque fois : listes recréées). */
  function controls(name) {
    return [...panels[name].element.querySelectorAll("input, select, textarea")].filter(
      (control) => !control.closest("[data-untracked]") && control.type !== "button",
    );
  }

  const panelOf = (control) => control.closest(".panel[data-panel]")?.dataset.panel;

  /** Les valeurs actuelles deviennent les valeurs appliquées. */
  function markClean(name) {
    const panel = panels[name];
    panel.snapshot = new Map(controls(name).map((control) => [control, valueOf(control)]));
    refresh(name);
  }

  /** Champs dont la valeur diffère de la valeur appliquée. */
  function changedControls(name) {
    const { snapshot } = panels[name];
    if (!snapshot) return [];
    return controls(name).filter((control) => !snapshot.has(control) || snapshot.get(control) !== valueOf(control));
  }

  const isDirty = (name) => changedControls(name).length > 0;

  /**
   * Valeur à envoyer pour `control` quand on applique la section `applying` :
   * la valeur saisie si le champ en fait partie, sinon la valeur appliquée
   * (les modifications des autres sections attendent leur propre « Appliquer »).
   */
  function valueFor(control, applying) {
    const name = panelOf(control);
    const snapshot = name && panels[name].snapshot;
    if (!applying || name === applying || !snapshot || !snapshot.has(control)) return valueOf(control);
    return snapshot.get(control);
  }

  /** Saisies en attente des sections données, pour les remettre après un remplissage. */
  function pendingValues(names) {
    const values = new Map();
    for (const name of names) {
      for (const control of changedControls(name)) {
        if (control.id) values.set(control.id, valueOf(control));
        else if (control.type === "radio") values.set(`radio:${control.name}:${control.value}`, control.checked);
      }
    }
    return values;
  }

  function restorePending(values) {
    for (const [key, value] of values) {
      const control = key.startsWith("radio:")
        ? document.querySelector(`input[name="${key.split(":")[1]}"][value="${key.split(":")[2]}"]`)
        : $(key);
      if (control) setValue(control, value);
    }
  }

  function applyState(name) {
    const panel = panels[name];
    const box = panel.element.querySelector(".apply-state");
    if (!box) return;
    // Un groupe de boutons radio compte pour une seule modification.
    const changed = new Set(
      changedControls(name).map((control) => (control.type === "radio" ? `radio:${control.name}` : control)),
    ).size;
    if (!box.firstChild) {
      const label = document.createElement("span");
      label.className = "state-label";
      const revert = document.createElement("button");
      revert.type = "button";
      revert.className = "revert";
      revert.textContent = "Annuler";
      revert.title = "Revenir aux réglages appliqués";
      revert.addEventListener("click", () => revertPanel(name));
      const apply = document.createElement("button");
      apply.type = "submit";
      apply.className = "primary apply";
      apply.textContent = "Appliquer";
      apply.title = "Appliquer cette section (Ctrl+S)";
      box.append(label, revert, apply);
    }
    const [label, revert, apply] = box.children;
    box.classList.toggle("dirty", changed > 0);
    label.textContent = changed
      ? `${changed} modification${changed > 1 ? "s" : ""} non appliquée${changed > 1 ? "s" : ""}`
      : "✓ Réglages appliqués";
    revert.hidden = !changed;
    apply.disabled = !changed || panel.busy;
  }

  /** Met à jour les signes « modifié » d'une section. */
  function refresh(name) {
    const panel = panels[name];
    if (!panel) return;
    const changed = new Set(changedControls(name));
    for (const field of panel.element.querySelectorAll(".field")) {
      const inside = [...field.querySelectorAll("input, select, textarea")];
      if (field.matches("input, select, textarea")) inside.push(field);
      field.classList.toggle("changed", inside.some((control) => changed.has(control)));
    }
    applyState(name);
    const dirty = changed.size > 0;
    document.querySelector(`[data-panel-link="${name}"]`)?.classList.toggle("dirty", dirty);
    $("nav-settings-dot")?.classList.toggle("on", Object.keys(panels).some((n) => isDirty(n)));
  }

  function showResult(name, text, isError = false) {
    const result = panels[name].element.querySelector(".panel-result");
    if (!result) return;
    result.hidden = false;
    result.textContent = text;
    result.classList.toggle("error", isError);
  }

  function revertPanel(name) {
    const panel = panels[name];
    if (panel.revert) panel.revert();
    else {
      for (const [control, value] of panel.snapshot ?? []) setValue(control, value);
    }
    refresh(name);
    showResult(name, "Modifications annulées : réglages appliqués remis.");
  }

  async function applyPanel(name) {
    const panel = panels[name];
    if (!panel.apply || panel.busy || !isDirty(name)) return;
    panel.busy = true;
    applyState(name);
    showResult(name, "Application…");
    try {
      const message = await panel.apply(name);
      markClean(name);
      showResult(name, message ?? "✓ Appliqué, sans redémarrage.");
    } catch (e) {
      showResult(name, String(e), true);
    } finally {
      panel.busy = false;
      applyState(name);
    }
  }

  for (const [name, panel] of Object.entries(panels)) {
    const update = () => refresh(name);
    panel.element.addEventListener("input", update);
    panel.element.addEventListener("change", update);
    // Valeurs changées par un bouton (préréglage, modèle proposé par le test).
    panel.element.addEventListener("click", () => setTimeout(update));
    if (panel.element.tagName === "FORM") {
      panel.element.addEventListener("submit", (event) => {
        event.preventDefault();
        applyPanel(name);
      });
      // Entrée dans un champ non suivi (clé API, nom de profil) : pas d'« Appliquer ».
      panel.element.addEventListener("keydown", (event) => {
        if (event.key === "Enter" && event.target.closest("[data-untracked]") && event.target.tagName === "INPUT") {
          event.preventDefault();
        }
      });
    }
  }

  /** Affiche la section `name` (ou la dernière vue). */
  function show(name) {
    let wanted = name;
    if (!panels[wanted]) {
      try {
        wanted = localStorage.getItem(LAST_PANEL_KEY);
      } catch {
        wanted = null;
      }
    }
    if (!panels[wanted]) wanted = DEFAULT_PANEL;
    try {
      localStorage.setItem(LAST_PANEL_KEY, wanted);
    } catch {
      // Pas de stockage : on revient sur la première section.
    }
    for (const [other, panel] of Object.entries(panels)) panel.element.hidden = other !== wanted;
    for (const link of document.querySelectorAll("[data-panel-link]")) {
      if (link.dataset.panelLink === wanted) link.setAttribute("aria-current", "true");
      else link.removeAttribute("aria-current");
    }
    return wanted;
  }

  const shownPanel = () => Object.keys(panels).find((name) => !panels[name].element.hidden);

  // Ctrl+S : applique la section affichée.
  document.addEventListener("keydown", (event) => {
    if (!(event.ctrlKey && event.key.toLowerCase() === "s") || currentPage() !== "reglages") return;
    event.preventDefault();
    const name = shownPanel();
    const form = name && panels[name].element;
    if (form?.tagName === "FORM") form.requestSubmit(form.querySelector("button.apply") ?? undefined);
  });

  return {
    /** Comment appliquer / annuler une section. */
    define(name, { apply, revert }) {
      panels[name].apply = apply;
      panels[name].revert = revert;
    },
    markClean,
    refresh,
    isDirty,
    valueFor,
    pendingValues,
    restorePending,
    showResult,
    show,
    panelsOfKind: (kind) => Object.keys(panels).filter((name) => panels[name].element.dataset.kind === kind),
  };
})();
