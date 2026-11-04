// Pastille d'état : écoute, transcription, chargement, erreur.
const pill = document.getElementById("pill");
const label = document.getElementById("label");
const LABELS = { listening: "Écoute…", processing: "Transcription…" };
// Mains libres : le message (nom du raccourci) sert au menu ; la pastille reste courte.
const SHORT = { handsfree: "Mains libres · Échap annule", command: "Commande…" };
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
