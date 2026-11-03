// Historique des dictées : liste, recherche, coût par entrée, copie, effacement.
const historyDateFormat = new Intl.DateTimeFormat("fr-FR", {
  dateStyle: "short",
  timeStyle: "short",
});
let historyEntries = [];

async function showHistory() {
  try {
    const view = await invoke("history_view");
    historyEntries = view.entries;
    $("history-disabled").hidden = view.enabled;
    $("history-clear").disabled = historyEntries.length === 0;
    renderHistory();
  } catch (e) {
    $("history-empty").hidden = false;
    $("history-empty").textContent = `Lecture impossible : ${e}`;
  }
}

function renderHistory() {
  const query = $("history-search").value.trim().toLowerCase();
  const shown = query
    ? historyEntries.filter((e) => `${e.text}\n${e.raw}`.toLowerCase().includes(query))
    : historyEntries;
  $("history-list").replaceChildren(...shown.map(renderEntry));
  $("history-empty").hidden = shown.length > 0;
  $("history-empty").textContent = query ? "Aucun résultat." : "Aucune dictée pour l'instant.";
}

function renderEntry(entry) {
  const item = $("history-entry").content.firstElementChild.cloneNode(true);
  const meta = [
    historyDateFormat.format(new Date(entry.at_ms)),
    `${Math.round(entry.duration_ms / 1000)} s`,
  ];
  if (entry.app) meta.push(entry.app);
  if (entry.cost != null) meta.push(formatCost(entry.cost, entry.currency ?? "USD"));
  item.querySelector(".when").textContent = meta.join(" · ");
  if (entry.usage) {
    item.querySelector(".when").title =
      `${integer.format(entry.usage.input_tokens)} jetons envoyés, ` +
      `${integer.format(entry.usage.output_tokens)} reçus`;
  }
  item.querySelector(".badge").textContent = entry.command
    ? "commande"
    : entry.snippet
      ? "snippet"
      : entry.cleanup_failed
        ? "non nettoyé"
        : "";
  item.querySelector(".text").textContent = entry.text;
  const raw = item.querySelector("details");
  if (entry.command) raw.querySelector("summary").textContent = "Consigne";
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

$("history-search").addEventListener("input", renderHistory);

// Effacement en deux clics : le premier arme le bouton pendant 4 s.
let clearArmed = null;
$("history-clear").addEventListener("click", async () => {
  const button = $("history-clear");
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
    await showHistory();
  }
});

registerPage("historique", { show: showHistory });
