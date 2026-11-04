// Pastille d'état : écoute, transcription, chargement, erreur.
const pill = document.getElementById("pill");
const label = document.getElementById("label");
const LABELS = { listening: "Écoute…", processing: "Transcription…" };
// Mains libres : le message (nom du raccourci) sert au menu ; la pastille reste courte.
const SHORT = { handsfree: "Mains libres · Échap annule", command: "Commande…" };
const ERROR_VISIBLE_MS = 5000;
const INFO_VISIBLE_MS = 1500;
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

function showLanguage() {
  const recording = ["listening", "handsfree"].includes(pill.dataset.state);
  lang.hidden = !(translation && recording);
  lang.textContent = translation ? `→ ${LANGUAGE_CODES[translation] ?? translation}` : "";
}

function render(status) {
  clearTimeout(errorTimer);
  lastStatus = status;
  pill.dataset.state = status.state;
  label.textContent = SHORT[status.state] ?? status.message ?? LABELS[status.state] ?? "";
  if (status.state === "error") {
    errorTimer = setTimeout(() => (pill.dataset.state = "idle"), ERROR_VISIBLE_MS);
  }
  showLanguage();
}

window.__TAURI__.event.listen("status", (event) => render(event.payload));

// Mode traduction activé ou coupé : confirmation brève si la pastille est au repos.
window.__TAURI__.event.listen("translation", (event) => {
  translation = event.payload;
  showLanguage();
  if (lastStatus.state !== "idle") return;
  clearTimeout(errorTimer);
  pill.dataset.state = "info";
  label.textContent = translation ? `Traduction → ${translation}` : "Traduction coupée";
  errorTimer = setTimeout(() => (pill.dataset.state = "idle"), INFO_VISIBLE_MS);
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
window.__TAURI__.core.invoke("translation_state").then((language) => {
  translation = language;
  showLanguage();
});
