// Pastille d'état : écoute, transcription, chargement, erreur.
const pill = document.getElementById("pill");
const label = document.getElementById("label");
const LABELS = { listening: "Écoute…", processing: "Transcription…" };
// Mains libres : le message (nom du raccourci) sert au menu ; la pastille reste courte.
const SHORT = { handsfree: "Mains libres · Échap annule", command: "Commande…" };
const ERROR_VISIBLE_MS = 5000;
const INFO_VISIBLE_MS = 1500;
const CONSENT_VISIBLE_MS = 5000;
let errorTimer;

// Mode traduction : langue d'arrivée affichée pendant la dictée.
const lang = document.getElementById("lang");
const LANGUAGE_CODES = {
  anglais: "EN",
  espagnol: "ES",
  allemand: "DE",
  italien: "IT",
  portugais: "PT",
  néerlandais: "NL",
};
let translation = null;
let lastStatus = { state: "idle" };
/** Début de la réunion enregistrée (horloge locale), sinon `null`. */
let meetingStartedAt = null;

function showLanguage() {
  const recording = ["listening", "handsfree"].includes(pill.dataset.state);
  lang.hidden = !(translation && recording);
  lang.textContent = translation ? `→ ${LANGUAGE_CODES[translation] ?? translation}` : "";
}

/** Au repos : rien, ou la réunion en cours d'enregistrement et sa durée. */
function showIdle() {
  if (meetingStartedAt == null) {
    pill.dataset.state = "idle";
    return;
  }
  const seconds = Math.floor((Date.now() - meetingStartedAt) / 1000);
  const h = Math.floor(seconds / 3600);
  const m = String(Math.floor(seconds / 60) % 60).padStart(2, "0");
  const s = String(seconds % 60).padStart(2, "0");
  pill.dataset.state = "meeting";
  label.textContent = `Réunion ${h > 0 ? `${h}:${m}:${s}` : `${m}:${s}`}`;
}
setInterval(() => pill.dataset.state === "meeting" && showIdle(), 1000);

function render(status) {
  clearTimeout(errorTimer);
  lastStatus = status;
  if (status.state === "idle") {
    showIdle();
    showLanguage();
    return;
  }
  pill.dataset.state = status.state;
  label.textContent = SHORT[status.state] ?? status.message ?? LABELS[status.state] ?? "";
  if (status.state === "error") {
    errorTimer = setTimeout(showIdle, ERROR_VISIBLE_MS);
  }
  showLanguage();
}

/** Message bref quand la pastille est au repos (ou affiche la réunion). */
function flash(text, state, duration) {
  if (!["idle", "meeting"].includes(pill.dataset.state)) return;
  clearTimeout(errorTimer);
  pill.dataset.state = state;
  label.textContent = text;
  errorTimer = setTimeout(showIdle, duration);
}

window.__TAURI__.event.listen("status", (event) => render(event.payload));

// Mode traduction activé ou coupé : confirmation brève si la pastille est au repos.
window.__TAURI__.event.listen("translation", (event) => {
  translation = event.payload;
  showLanguage();
  if (lastStatus.state !== "idle") return;
  flash(translation ? `Traduction → ${translation}` : "Traduction coupée", "info", INFO_VISIBLE_MS);
});

// Réunion démarrée ou arrêtée.
function setMeeting(state) {
  meetingStartedAt = state.recording ? Date.now() - state.elapsed_ms : null;
  if (lastStatus.state !== "idle") return;
  if (state.consent) {
    showIdle();
    flash("Réunion : prévenez les participants", "info", CONSENT_VISIBLE_MS);
  } else {
    clearTimeout(errorTimer);
    showIdle();
  }
}
window.__TAURI__.event.listen("meeting", (event) => setMeeting(event.payload));
window.__TAURI__.event.listen("meeting-error", (event) => {
  if (lastStatus.state === "idle") flash(String(event.payload), "error", ERROR_VISIBLE_MS);
});

// Niveau du micro (0 à 1) pendant l'enregistrement : barres plus hautes au centre.
const bars = [...document.querySelectorAll("#meter i")];
const WEIGHTS = [0.55, 0.8, 1, 0.8, 0.55];
window.__TAURI__.event.listen("level", (event) => {
  const level = Math.max(0, Math.min(1, Number(event.payload) || 0));
  bars.forEach((bar, i) => {
    bar.style.transform = `scaleY(${Math.max(0.2, level * WEIGHTS[i])})`;
  });
});
// Rattrape l'état émis avant le chargement de la page (téléchargement du modèle…).
window.__TAURI__.core.invoke("current_status").then(render);
window.__TAURI__.core.invoke("meeting_state").then(setMeeting);
window.__TAURI__.core.invoke("translation_state").then((language) => {
  translation = language;
  showLanguage();
});
