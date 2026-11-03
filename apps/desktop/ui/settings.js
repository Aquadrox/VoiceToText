// Réglages : clé API du LLM (enregistrement, suppression, test), consommation.
const { invoke } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);
let currency = "USD";

async function refresh() {
  const view = await invoke("settings_view");
  $("model").textContent = `${view.model} (${view.base_url})`;
  $("disabled").hidden = view.llm_enabled;
  const status = $("key-status");
  status.textContent = view.has_api_key ? "✓ Clé enregistrée" : "Aucune clé enregistrée";
  status.classList.toggle("ok", view.has_api_key);
  $("delete").hidden = !view.has_api_key;
  $("test").disabled = !view.has_api_key;
  currency = view.currency;
  renderUsage(view.usage);
}

const integer = new Intl.NumberFormat("fr-FR");

function formatCost(value) {
  // Montants minuscules (une dictée ≈ 0,0001 $) : chiffres significatifs.
  const tiny = value > 0 && value < 0.01;
  const precision = tiny ? { maximumSignificantDigits: 2 } : { minimumFractionDigits: 2 };
  try {
    return new Intl.NumberFormat("fr-FR", { style: "currency", currency, ...precision }).format(value);
  } catch {
    return `${tiny ? value.toPrecision(2) : value.toFixed(2)} ${currency}`;
  }
}

function renderUsage(usage) {
  for (const row of document.querySelectorAll("#usage tbody tr")) {
    const totals = usage[row.dataset.period];
    const [requests, tokens, cost] = row.querySelectorAll("td");
    requests.textContent = integer.format(totals.requests);
    tokens.textContent = integer.format(totals.input_tokens + totals.output_tokens);
    cost.textContent = formatCost(totals.cost);
  }
}

function showResult(text, isError = false) {
  const result = $("result");
  result.hidden = false;
  result.textContent = text;
  result.classList.toggle("error", isError);
}

async function busy(button, action) {
  button.disabled = true;
  try {
    await action();
  } catch (e) {
    showResult(String(e), true);
  } finally {
    button.disabled = false;
    await refresh();
  }
}

$("save").addEventListener("click", () =>
  busy($("save"), async () => {
    await invoke("save_api_key", { key: $("key").value });
    $("key").value = "";
    showResult("Clé enregistrée. Cliquez sur « Tester » pour la vérifier.");
  }),
);

$("key").addEventListener("keydown", (e) => {
  if (e.key === "Enter") $("save").click();
});

$("delete").addEventListener("click", () =>
  busy($("delete"), async () => {
    await invoke("delete_api_key");
    showResult("Clé supprimée : le texte brut de Whisper sera inséré.");
  }),
);

$("test").addEventListener("click", () =>
  busy($("test"), async () => {
    showResult("Test en cours…");
    const t = await invoke("test_cleanup");
    const tokens = t.usage
      ? ` · ${t.usage.input_tokens} + ${t.usage.output_tokens} jetons · ${formatCost(t.cost)}`
      : "";
    showResult(`Avant : ${t.input}\n\nAprès : ${t.output}\n\n${t.duration_ms} ms${tokens}`);
  }),
);

refresh();
