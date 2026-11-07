// Zone de test en bas des réglages : dicter dans le champ (ou ailleurs) et
// voir aussitôt ce que les réglages appliqués produisent : ce que le moteur
// local a entendu, le texte inséré, la durée de chaque étape, le coût.
// Le texte dicté est inséré avec textContent (jamais innerHTML) et n'est
// gardé que dans la page.
const TEST_COLLAPSED_KEY = "vtt-test-collapsed";
const seconds = new Intl.NumberFormat("fr-FR", { maximumFractionDigits: 1 });

const STATE_LABELS = {
  idle: "Prêt",
  listening: "● Écoute…",
  handsfree: "● Mains libres",
  command: "● Consigne…",
  processing: "Transcription…",
};

function renderTestState(status) {
  const element = $("test-state");
  element.dataset.state = status.state;
  element.textContent =
    status.state === "loading"
      ? status.message
      : status.state === "error"
        ? `⚠ ${status.message}`
        : (STATE_LABELS[status.state] ?? status.state);
}

/** Libellé lisible d'un modèle (fichier → « Parakeet TDT v3 »). */
function modelLabel(file) {
  const model = generalView?.models.find((m) => m.file === file);
  return model ? model.label : file;
}

/** Hôte d'une adresse d'API, pour un résumé court. */
function hostOf(url) {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

/** Résumé des réglages appliqués, au-dessus du champ de test. */
function renderTestConfig() {
  const parts = [];
  if (generalView) {
    parts.push(`Touche : ${keyLabel(generalView.hotkey.key)}`);
    parts.push(`Moteur : ${modelLabel(generalView.asr.model)}`);
  }
  if (activeLlm) {
    parts.push(
      activeLlm.enabled
        ? `Nettoyage : ${activeLlm.model || "?"} (${hostOf(activeLlm.base_url)})`
        : "Nettoyage : désactivé (texte brut)",
    );
  }
  $("test-config").textContent = parts.join(" · ");
}

function step(text, kind = "") {
  const item = document.createElement("li");
  if (kind) item.className = kind;
  item.textContent = text;
  return item;
}

const ms = (value) => (value >= 1000 ? `${seconds.format(value / 1000)} s` : `${value} ms`);

function renderReport(r) {
  $("test-empty").hidden = true;
  $("test-report").hidden = false;
  const steps = [step(`Audio ${seconds.format(r.audio_ms / 1000)} s${r.app ? ` · ${r.app}` : ""}`)];
  if (r.asr_ms > 0) steps.push(step(`Transcription ${ms(r.asr_ms)} · ${modelLabel(r.model)}`, "ok"));
  const after = Math.max(0, r.total_ms - r.asr_ms);
  const warning = $("test-warning");
  warning.hidden = true;

  switch (r.kind) {
    case "skipped":
      steps.push(step("Rien inséré : trop court, silence ou bruit sans parole", "warn"));
      break;
    case "undone":
      steps.push(step("Commande vocale : dernière dictée annulée (Ctrl+Z)", "ok"));
      break;
    case "error":
      steps.push(step("Échec", "fail"));
      warning.hidden = false;
      warning.textContent = r.error;
      break;
    default: {
      let llm;
      if (r.snippet) llm = step("Snippet : texte fixe, sans LLM", "ok");
      else if (r.cleanup_error) llm = step(`Nettoyage échoué (${ms(after)}) : texte brut inséré`, "fail");
      else if (r.input_tokens != null) {
        const via = r.fallback ? ` via le secours « ${r.fallback} »` : "";
        const label = r.command ? "Command Mode" : r.translated ? `Traduction (${r.translated})` : "Nettoyage";
        llm = step(
          `${label} ${ms(after)}${via} · ${integer.format(r.input_tokens)} + ${integer.format(r.output_tokens)} jetons · ${formatCost(r.cost ?? 0, r.currency ?? "USD")}`,
          r.fallback ? "warn" : "ok",
        );
      } else llm = step(`Sans nettoyage (texte brut) · ${ms(after)}`);
      steps.push(llm);
      steps.push(step(r.retry ? "Réessayée" : `Inséré · total ${ms(r.total_ms)}`, "ok"));
      if (r.cleanup_error) {
        warning.hidden = false;
        warning.textContent = `Nettoyage : ${r.cleanup_error}`;
      }
    }
  }
  $("test-steps").replaceChildren(...steps);
  $("test-raw").textContent = r.raw || "—";
  $("test-text").textContent = r.text || "—";
  $("test-text-label").textContent = r.command ? "Résultat de la consigne" : "Inséré";
}

function setCollapsed(collapsed) {
  $("test-body").hidden = collapsed;
  $("test-dock").classList.toggle("collapsed", collapsed);
  $("test-toggle").textContent = collapsed ? "Ouvrir" : "Réduire";
  $("test-toggle").setAttribute("aria-expanded", String(!collapsed));
  try {
    localStorage.setItem(TEST_COLLAPSED_KEY, collapsed ? "1" : "");
  } catch {
    // Pas de stockage : état par défaut au prochain affichage.
  }
}

$("test-toggle").addEventListener("click", () => setCollapsed(!$("test-body").hidden));
$("test-clear").addEventListener("click", () => {
  $("test-input").value = "";
  $("test-report").hidden = true;
  $("test-empty").hidden = false;
  $("test-input").focus();
});

try {
  setCollapsed(localStorage.getItem(TEST_COLLAPSED_KEY) === "1");
} catch {
  setCollapsed(false);
}

window.__TAURI__.event.listen("status", (event) => renderTestState(event.payload));
window.__TAURI__.event.listen("dictation", (event) => renderReport(event.payload));
window.addEventListener("settings-applied", renderTestConfig);
invoke("current_status")
  .then(renderTestState)
  .catch(() => {});
