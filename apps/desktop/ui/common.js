// Outils partagés par les pages de la fenêtre principale.
// Le texte dicté est toujours inséré avec textContent (jamais innerHTML).
const { invoke } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);
const integer = new Intl.NumberFormat("fr-FR");

/** Pages enregistrées : { nom: { show() } } ; `show` est appelé à l'affichage. */
const PAGES = {};
const registerPage = (name, page) => (PAGES[name] = page);
/** Ancre « #page/section » : la page, puis la section (Réglages). */
const currentPage = () => location.hash.slice(1).split("/")[0] || "accueil";
const currentSection = () => location.hash.slice(1).split("/")[1] ?? "";
const isShown = (name) => !document.hidden && currentPage() === name;

function formatCost(value, currency) {
  // Montants minuscules (une dictée ≈ 0,0001 $) : chiffres significatifs.
  const tiny = value > 0 && value < 0.01;
  const precision = tiny ? { maximumSignificantDigits: 2 } : { minimumFractionDigits: 2 };
  try {
    return new Intl.NumberFormat("fr-FR", { style: "currency", currency, ...precision }).format(value);
  } catch {
    return `${tiny ? value.toPrecision(2) : value.toFixed(2)} ${currency}`;
  }
}

/** Durée lisible : « 45 s », « 12 min », « 3 h 05 ». */
function formatDuration(ms) {
  const minutes = Math.round(ms / 60000);
  if (ms < 60000) return `${Math.round(ms / 1000)} s`;
  if (minutes < 60) return `${minutes} min`;
  return `${Math.floor(minutes / 60)} h ${String(minutes % 60).padStart(2, "0")}`;
}
