// Réglages : clé API du LLM (enregistrement, suppression, test).
const { invoke } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);

async function refresh() {
  const view = await invoke("settings_view");
  $("model").textContent = `${view.model} (${view.base_url})`;
  $("disabled").hidden = view.llm_enabled;
  const status = $("key-status");
  status.textContent = view.has_api_key ? "✓ Clé enregistrée" : "Aucune clé enregistrée";
  status.classList.toggle("ok", view.has_api_key);
  $("delete").hidden = !view.has_api_key;
  $("test").disabled = !view.has_api_key;
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
    showResult(`Avant : ${t.input}\n\nAprès : ${t.output}\n\n${t.duration_ms} ms`);
  }),
);

refresh();
