// Pastille d'état : écoute, transcription, chargement, erreur.
const pill = document.getElementById("pill");
const label = document.getElementById("label");
const LABELS = { listening: "Écoute…", processing: "Transcription…" };
// Mains libres : le message (nom du raccourci) sert au menu ; la pastille reste courte.
const SHORT = { handsfree: "Mains libres · Échap annule" };
const ERROR_VISIBLE_MS = 5000;
let errorTimer;

function render(status) {
  clearTimeout(errorTimer);
  pill.dataset.state = status.state;
  label.textContent = SHORT[status.state] ?? status.message ?? LABELS[status.state] ?? "";
  if (status.state === "error") {
    errorTimer = setTimeout(() => (pill.dataset.state = "idle"), ERROR_VISIBLE_MS);
  }
}

window.__TAURI__.event.listen("status", (event) => render(event.payload));
// Rattrape l'état émis avant le chargement de la page (téléchargement du modèle…).
window.__TAURI__.core.invoke("current_status").then(render);
