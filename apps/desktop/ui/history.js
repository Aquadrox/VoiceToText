// Historique des dictées : liste, recherche, copie, effacement.
// Le texte dicté est inséré avec textContent uniquement (jamais innerHTML).
const { invoke } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);

const dateFormat = new Intl.DateTimeFormat("fr-FR", {
  dateStyle: "short",
  timeStyle: "short",
});
let entries = [];

async function refresh() {
  try {
    const view = await invoke("history_view");
    entries = view.entries;
    $("disabled").hidden = view.enabled;
    $("clear").disabled = entries.length === 0;
    render();
  } catch (e) {
    $("empty").hidden = false;
    $("empty").textContent = `Lecture impossible : ${e}`;
  }
}

function render() {
  const query = $("search").value.trim().toLowerCase();
  const shown = query
    ? entries.filter((e) => `${e.text}\n${e.raw}`.toLowerCase().includes(query))
    : entries;
  const list = $("list");
  list.replaceChildren(...shown.map(renderEntry));
  $("empty").hidden = shown.length > 0;
  $("empty").textContent = query ? "Aucun résultat." : "Aucune dictée pour l'instant.";
}

function renderEntry(entry) {
  const item = $("entry").content.firstElementChild.cloneNode(true);
  const seconds = Math.round(entry.duration_ms / 1000);
  item.querySelector(".when").textContent = `${dateFormat.format(new Date(entry.at_ms))} · ${seconds} s`;
  item.querySelector(".badge").textContent = entry.snippet
    ? "snippet"
    : entry.cleanup_failed
      ? "non nettoyé"
      : "";
  item.querySelector(".text").textContent = entry.text;
  const raw = item.querySelector("details");
  if (entry.raw === entry.text) raw.remove();
  else item.querySelector(".raw").textContent = entry.raw;

  const copy = item.querySelector(".copy");
  copy.addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(entry.text);
      copy.textContent = "Copié ✓";
    } catch {
      copy.textContent = "Échec";
    }
    setTimeout(() => (copy.textContent = "Copier"), 1500);
  });
  return item;
}

$("search").addEventListener("input", render);

// Effacement en deux clics : le premier arme le bouton pendant 4 s.
let clearArmed = null;
$("clear").addEventListener("click", async () => {
  const button = $("clear");
  if (!clearArmed) {
    button.textContent = "Confirmer l'effacement";
    clearArmed = setTimeout(() => {
      clearArmed = null;
      button.textContent = "Tout effacer";
    }, 4000);
    return;
  }
  clearTimeout(clearArmed);
  clearArmed = null;
  button.textContent = "Tout effacer";
  try {
    await invoke("clear_history");
  } finally {
    await refresh();
  }
});

// Les nouvelles dictées apparaissent quand on revient sur la fenêtre.
window.addEventListener("focus", refresh);
refresh();
