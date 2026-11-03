// Réglages : fournisseur LLM (appliqué sans redémarrer), profils, clé API,
// test avec diagnostic, consommation.
let currency = "USD";
let profiles = [];
/** Configuration actuellement appliquée (pour repérer le profil actif). */
let activeLlm = null;

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
    label: "Ollama (ce PC ou réseau local)",
    base_url: "http://localhost:11434/v1",
    model: "",
    timeout_ms: 15000,
    pricing: UNKNOWN_PRICE,
    extra: {},
    note: "Pas de clé ni de coût. Sur une autre machine : http://ADRESSE-IP:11434/v1 (Ollama lancé avec OLLAMA_HOST=0.0.0.0). « Tester ces réglages » liste les modèles installés.",
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
  refreshKey();
}

/* ---------- Profils ---------- */

const sameLlm = (a, b) => a && b && JSON.stringify(a) === JSON.stringify(b);

function renderProfiles() {
  const select = $("profile");
  const active = profiles.find((p) => sameLlm(stripName(p), activeLlm));
  const options = [option("", active ? "— Réglages du formulaire —" : "— Réglages actuels (non enregistrés en profil) —")];
  for (const profile of profiles) {
    const label = profile === active ? `${profile.name} (actif)` : profile.name;
    options.push(option(profile.name, label));
  }
  select.replaceChildren(...options);
  select.value = active ? active.name : "";
  $("profile-delete").disabled = !select.value;
}

/** Profil sans son nom : la forme d'une configuration LLM. */
function stripName(profile) {
  const { name, ...llm } = profile;
  return llm;
}

$("profile").addEventListener("change", () => {
  const profile = profiles.find((p) => p.name === $("profile").value);
  $("profile-delete").disabled = !profile;
  if (!profile) return;
  fillProvider(stripName(profile));
  $("profile-name").value = profile.name;
  showProviderResult(`Profil « ${profile.name} » chargé : « Appliquer » pour l'activer.`);
});

async function saveProfiles(next, message) {
  try {
    profiles = await invoke("save_llm_profiles", { profiles: next });
    renderProfiles();
    showProviderResult(message);
  } catch (e) {
    showProviderResult(String(e), true);
  }
}

$("profile-save").addEventListener("click", async () => {
  const name = $("profile-name").value.trim();
  if (!name) {
    showProviderResult("Donnez un nom au profil.", true);
    return;
  }
  let llm;
  try {
    llm = readProvider();
  } catch (e) {
    showProviderResult(String(e), true);
    return;
  }
  const others = profiles.filter((p) => p.name.toLowerCase() !== name.toLowerCase());
  await saveProfiles([...others, { name, ...llm }], `Profil « ${name} » enregistré.`);
  $("profile").value = name;
  $("profile-delete").disabled = false;
});

$("profile-delete").addEventListener("click", async () => {
  const name = $("profile").value;
  if (!name) return;
  await saveProfiles(
    profiles.filter((p) => p.name !== name),
    `Profil « ${name} » supprimé (les réglages actifs ne changent pas).`,
  );
});

/* ---------- Clé de l'adresse saisie ---------- */

let keyTimer = null;
async function refreshKey() {
  const url = $("base-url").value.trim();
  try {
    const status = await invoke("key_status", { baseUrl: url });
    $("key-host").textContent = status.host ? `(${status.host})` : "";
    const line = $("key-status");
    line.textContent = status.cleartext
      ? status.has_key
        ? "⚠ Clé enregistrée, mais non envoyée : adresse HTTP hors de ce PC"
        : "Adresse HTTP hors de ce PC : aucune clé n'y sera envoyée (Ollama n'en a pas besoin)"
      : status.has_key
        ? "✓ Clé enregistrée"
        : status.optional
          ? "Aucune clé : facultative pour un serveur local"
          : "Aucune clé enregistrée";
    line.classList.toggle("ok", status.has_key && !status.cleartext);
    $("delete").hidden = !status.has_key;
  } catch {
    $("key-status").textContent = "";
  }
}

$("base-url").addEventListener("input", () => {
  clearTimeout(keyTimer);
  keyTimer = setTimeout(refreshKey, 400);
});

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

function showProviderResult(text, isError = false) {
  const result = $("provider-result");
  result.hidden = false;
  result.textContent = text;
  result.classList.toggle("error", isError);
}

$("provider-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const button = $("save-provider");
  button.disabled = true;
  try {
    const result = await invoke("apply_llm_settings", { llm: readProvider() });
    activeLlm = result.llm;
    currency = result.llm.pricing.currency;
    fillProvider(result.llm);
    renderProfiles();
    showProviderResult(
      result.applied
        ? "✓ Appliqué : utilisé dès la prochaine dictée."
        : "✓ Enregistré : appliqué une fois le modèle chargé (au prochain démarrage).",
    );
  } catch (e) {
    showProviderResult(String(e), true);
  } finally {
    button.disabled = false;
  }
});

/* ---------- Test avec diagnostic ---------- */

function row(list, term, value) {
  const dt = document.createElement("dt");
  dt.textContent = term;
  const dd = document.createElement("dd");
  if (value instanceof Node) dd.append(value);
  else dd.textContent = value;
  list.append(dt, dd);
}

function renderDiagnostic(d) {
  const panel = $("diagnostic");
  panel.hidden = false;
  panel.className = `diagnostic ${d.ok ? "ok" : "failed"}`;
  const title = document.createElement("h3");
  title.textContent = d.ok ? `✓ Réponse reçue en ${d.duration_ms} ms` : "✗ Échec du test";
  const list = document.createElement("dl");
  row(list, "Adresse appelée", `POST ${d.endpoint}`);
  if (d.corrected_url) row(list, "Adresse corrigée", `${d.corrected_url} (appliquée si vous validez)`);
  row(list, "Modèle", d.model);
  row(list, "Clé", d.key);
  row(list, "Phrase envoyée", d.input);
  if (d.output != null) row(list, "Réponse", d.output);
  if (d.error) row(list, "Erreur", d.error);
  if (d.usage) {
    row(
      list,
      "Jetons",
      `${integer.format(d.usage.input_tokens)} envoyés (dont ${integer.format(d.usage.cached_input_tokens)} en cache), ` +
        `${integer.format(d.usage.output_tokens)} reçus · ${formatCost(d.cost ?? 0, d.currency)}`,
    );
  }
  if (d.models.length) {
    const models = document.createElement("div");
    models.className = "models";
    for (const name of d.models) {
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = name;
      button.title = "Utiliser ce modèle";
      button.addEventListener("click", () => {
        $("model").value = name;
        showProviderResult(`Modèle « ${name} » choisi : retestez puis « Appliquer ».`);
      });
      models.append(button);
    }
    row(list, `Modèles du serveur (${d.models.length})`, models);
  } else if (d.models_error) {
    row(list, "Liste des modèles", `indisponible : ${d.models_error}`);
  }
  const content = [title, list];
  if (d.hints.length) {
    const hints = document.createElement("ul");
    for (const hint of d.hints) {
      const item = document.createElement("li");
      item.textContent = hint;
      hints.append(item);
    }
    const label = document.createElement("strong");
    label.textContent = "Pistes";
    content.push(label, hints);
  }
  panel.replaceChildren(...content);
}

async function runTest(button) {
  let llm;
  try {
    llm = readProvider();
  } catch (e) {
    showProviderResult(String(e), true);
    return;
  }
  button.disabled = true;
  showProviderResult("Test en cours…");
  try {
    const diagnostic = await invoke("test_llm_settings", { llm, key: $("key").value || null });
    $("provider-result").hidden = true;
    renderDiagnostic(diagnostic);
  } catch (e) {
    showProviderResult(String(e), true);
  } finally {
    button.disabled = false;
  }
}

$("provider-test").addEventListener("click", () => runTest($("provider-test")));

async function refresh() {
  const view = await invoke("settings_view");
  currency = view.llm.pricing.currency;
  activeLlm = view.llm;
  profiles = view.profiles;
  renderUsage(view.usage);
  return view;
}

function renderUsage(usage) {
  for (const row of document.querySelectorAll("#usage tbody tr")) {
    const totals = usage[row.dataset.period];
    const [requests, tokens, cost] = row.querySelectorAll("td");
    requests.textContent = integer.format(totals.requests);
    tokens.textContent = integer.format(totals.input_tokens + totals.output_tokens);
    cost.textContent = formatCost(totals.cost, currency);
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
    await refreshKey();
  }
}

$("save").addEventListener("click", () =>
  busy($("save"), async () => {
    await invoke("save_api_key", { baseUrl: $("base-url").value.trim(), key: $("key").value });
    $("key").value = "";
    showResult("Clé enregistrée. Cliquez sur « Tester » pour la vérifier.");
  }),
);

$("key").addEventListener("keydown", (e) => {
  if (e.key === "Enter") $("save").click();
});

$("delete").addEventListener("click", () =>
  busy($("delete"), async () => {
    await invoke("delete_api_key", { baseUrl: $("base-url").value.trim() });
    showResult("Clé supprimée : le texte brut de Whisper sera inséré.");
  }),
);

// Même test que « Tester ces réglages » (résultat sous le formulaire).
$("test").addEventListener("click", () => runTest($("test")));

// Le formulaire n'est rempli qu'une fois : revenir sur la page ne doit pas
// écraser une saisie en cours.
let providerFilled = false;
registerPage("reglages", {
  show: () =>
    Promise.all([
      showGeneral(),
      refresh().then((view) => {
        if (!providerFilled) fillProvider(view.llm);
        providerFilled = true;
        renderProfiles();
      }),
    ]),
});

// Consommation en direct : les dictées se font dans d'autres fenêtres, la
// fenêtre ne reçoit donc pas le focus. Rafraîchie toutes les 2 s tant que la
// page est affichée (lecture locale, aucune requête réseau).
const USAGE_REFRESH_MS = 2000;
setInterval(async () => {
  if (!isShown("reglages")) return;
  try {
    const view = await invoke("settings_view");
    renderUsage(view.usage);
  } catch {
    // Prochain essai dans 2 s.
  }
}, USAGE_REFRESH_MS);
