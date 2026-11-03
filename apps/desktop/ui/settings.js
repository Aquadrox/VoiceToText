// Réglages : fournisseur LLM, clé API (enregistrement, suppression, test),
// consommation.
const { invoke } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);
let currency = "USD";

const NO_THINKING = { thinking: { type: "disabled" } };
const UNKNOWN_PRICE = { input_per_million: 0, cached_input_per_million: 0, output_per_million: 0 };

// Préréglages : ils remplissent le formulaire, tout reste modifiable.
const PRESETS = [
  {
    id: "deepseek",
    label: "DeepSeek",
    base_url: "https://api.deepseek.com",
    model: "deepseek-flash",
    timeout_ms: 3000,
    pricing: { input_per_million: 0.3, cached_input_per_million: 0.006, output_per_million: 1.2 },
    extra: NO_THINKING,
    note: "Tarif plein (heures de pointe) ; heures creuses à moitié prix.",
  },
  {
    id: "zai",
    label: "GLM (Z.ai)",
    base_url: "https://api.z.ai/api/paas/v4",
    model: "glm-5.2",
    timeout_ms: 4000,
    pricing: UNKNOWN_PRICE,
    extra: NO_THINKING,
    note: "La série GLM-5.3 ne permet pas de désactiver la réflexion : retirez le paramètre « thinking » si vous la choisissez. Tarif à renseigner.",
  },
  {
    id: "openai",
    label: "OpenAI",
    base_url: "https://api.openai.com/v1",
    model: "",
    timeout_ms: 4000,
    pricing: UNKNOWN_PRICE,
    extra: {},
    note: "Indiquez le nom exact du modèle (voir platform.openai.com) et son tarif.",
  },
  {
    id: "mistral",
    label: "Mistral",
    base_url: "https://api.mistral.ai/v1",
    model: "mistral-small-latest",
    timeout_ms: 4000,
    pricing: UNKNOWN_PRICE,
    extra: {},
    note: "Tarif à renseigner (voir mistral.ai).",
  },
  {
    id: "openrouter",
    label: "OpenRouter",
    base_url: "https://openrouter.ai/api/v1",
    model: "",
    timeout_ms: 5000,
    pricing: UNKNOWN_PRICE,
    extra: {},
    note: "Modèle au format « fournisseur/modèle ». Tarif à renseigner.",
  },
  {
    id: "ollama",
    label: "Ollama (sur ce PC)",
    base_url: "http://localhost:11434/v1",
    model: "",
    timeout_ms: 15000,
    pricing: UNKNOWN_PRICE,
    extra: {},
    note: "Modèle local : le texte ne quitte pas le PC, pas de clé ni de coût. Indiquez un modèle installé (ollama list).",
  },
  { id: "custom", label: "Autre (compatible OpenAI)", note: "Adresse, modèle et tarif libres." },
];

for (const preset of PRESETS) {
  const option = document.createElement("option");
  option.value = preset.id;
  option.textContent = preset.label;
  $("preset").append(option);
}

const formatExtra = (extra) => (Object.keys(extra).length ? JSON.stringify(extra) : "");

function fillProvider(llm) {
  $("enabled").checked = llm.enabled;
  $("base-url").value = llm.base_url;
  $("model").value = llm.model;
  $("timeout").value = llm.timeout_ms / 1000;
  $("price-input").value = llm.pricing.input_per_million;
  $("price-cached").value = llm.pricing.cached_input_per_million;
  $("price-output").value = llm.pricing.output_per_million;
  $("currency").value = llm.pricing.currency;
  $("extra").value = formatExtra(llm.extra_body);
  const preset = PRESETS.find((p) => p.base_url === llm.base_url) ?? PRESETS.at(-1);
  $("preset").value = preset.id;
  $("preset-note").textContent = preset.note;
}

$("preset").addEventListener("change", () => {
  const preset = PRESETS.find((p) => p.id === $("preset").value);
  $("preset-note").textContent = preset.note;
  if (preset.id === "custom") return;
  $("base-url").value = preset.base_url;
  $("model").value = preset.model;
  $("model").placeholder = preset.model ? "" : "nom du modèle";
  $("timeout").value = preset.timeout_ms / 1000;
  $("price-input").value = preset.pricing.input_per_million;
  $("price-cached").value = preset.pricing.cached_input_per_million;
  $("price-output").value = preset.pricing.output_per_million;
  $("extra").value = formatExtra(preset.extra);
});

function readProvider() {
  let extra = {};
  const text = $("extra").value.trim();
  if (text) {
    try {
      extra = JSON.parse(text);
    } catch (e) {
      throw `Paramètres supplémentaires : JSON invalide (${e.message})`;
    }
    if (typeof extra !== "object" || extra === null || Array.isArray(extra)) {
      throw "Paramètres supplémentaires : un objet JSON { … } est attendu";
    }
  }
  return {
    enabled: $("enabled").checked,
    base_url: $("base-url").value.trim().replace(/\/+$/, ""),
    model: $("model").value.trim(),
    timeout_ms: Math.round(Number($("timeout").value) * 1000),
    pricing: {
      input_per_million: Number($("price-input").value),
      cached_input_per_million: Number($("price-cached").value),
      output_per_million: Number($("price-output").value),
      currency: $("currency").value.trim().toUpperCase(),
    },
    extra_body: extra,
  };
}

$("provider-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const result = $("provider-result");
  const button = $("save-provider");
  button.disabled = true;
  try {
    await invoke("save_llm_settings", { llm: readProvider() });
    result.hidden = false;
    result.classList.remove("error");
    result.textContent = "Enregistré. Redémarrage de VoiceToText…";
  } catch (e) {
    result.hidden = false;
    result.classList.add("error");
    result.textContent = String(e);
    button.disabled = false;
  }
});

async function refresh() {
  const view = await invoke("settings_view");
  currency = view.llm.pricing.currency;
  $("key-host").textContent = view.key_host ? `(${view.key_host})` : "";
  const status = $("key-status");
  status.textContent = view.has_api_key
    ? "✓ Clé enregistrée"
    : view.key_optional
      ? "Aucune clé : facultative pour un modèle local"
      : "Aucune clé enregistrée";
  status.classList.toggle("ok", view.has_api_key);
  $("delete").hidden = !view.has_api_key;
  $("test").disabled = !view.has_api_key && !view.key_optional;
  renderUsage(view.usage);
  return view;
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

refresh().then((view) => fillProvider(view.llm));
