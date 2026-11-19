import test from "node:test";
import assert from "node:assert/strict";
import { createEnvironment } from "./dom.mjs";

function setupSettingsUI(opts = {}) {
  const env = createEnvironment({ hash: "#reglages", ...opts });
  const { el, document } = env;

  const navDot = el("span", { id: "nav-settings-dot" });
  const linkShortcuts = el("a", { attrs: { "data-panel-link": "raccourcis" } });
  const linkLlm = el("a", { attrs: { "data-panel-link": "nettoyage" } });

  // Section 1 : Raccourcis (champ texte + groupe radio + champ non suivi).
  const keyInput = el("input", { id: "hotkey-key", type: "text", value: "AltGr" });
  const radioA = el("input", { type: "radio", name: "mode", value: "ptt", checked: true });
  const radioB = el("input", { type: "radio", name: "mode", value: "toggle", checked: false });
  const untrackedInput = el("input", { id: "api-key", type: "password", value: "" });
  const untrackedBox = el("div", { attrs: { "data-untracked": "" } }, untrackedInput);
  const shortcutsApplyState = el("div", { className: "apply-state" });
  const shortcutsResult = el("p", { className: "panel-result", hidden: true });
  const shortcutsField1 = el("div", { className: "field" }, keyInput);
  const shortcutsField2 = el("div", { className: "field" }, radioA, radioB);
  const panelShortcuts = el(
    "form",
    { className: "panel", attrs: { "data-panel": "raccourcis", "data-kind": "general" } },
    shortcutsField1,
    shortcutsField2,
    untrackedBox,
    shortcutsApplyState,
    shortcutsResult,
  );

  // Section 2 : Nettoyage LLM (checkbox + modèle).
  const llmEnabled = el("input", { id: "llm-enabled", type: "checkbox", checked: true });
  const llmModel = el("input", { id: "llm-model", type: "text", value: "deepseek-chat" });
  const llmApplyState = el("div", { className: "apply-state" });
  const llmResult = el("p", { className: "panel-result", hidden: true });
  const panelLlm = el(
    "form",
    { className: "panel", attrs: { "data-panel": "nettoyage", "data-kind": "llm" } },
    el("div", { className: "field" }, llmEnabled),
    el("div", { className: "field" }, llmModel),
    llmApplyState,
    llmResult,
  );

  document.body.append(navDot, linkShortcuts, linkLlm, panelShortcuts, panelLlm);
  env.load("common.js", "settings-ui.js");

  const SettingsUI = env.eval("SettingsUI");
  SettingsUI.markClean("raccourcis");
  SettingsUI.markClean("nettoyage");

  return {
    env,
    SettingsUI,
    navDot,
    linkShortcuts,
    linkLlm,
    keyInput,
    radioA,
    radioB,
    untrackedInput,
    shortcutsApplyState,
    shortcutsResult,
    shortcutsField1,
    panelShortcuts,
    llmEnabled,
    llmModel,
    llmApplyState,
    llmResult,
    panelLlm,
  };
}

test("SettingsUI suit les champs modifiés, regroupe les radios et ignore data-untracked", () => {
  const {
    SettingsUI,
    navDot,
    linkShortcuts,
    keyInput,
    radioA,
    radioB,
    untrackedInput,
    shortcutsApplyState,
    shortcutsField1,
  } = setupSettingsUI();

  assert.equal(SettingsUI.isDirty("raccourcis"), false);
  assert.equal(navDot.classList.contains("on"), false);
  assert.equal(shortcutsApplyState.querySelector(".state-label").textContent, "✓ Réglages appliqués");
  assert.equal(shortcutsApplyState.querySelector(".apply").disabled, true);

  // Champ dans [data-untracked] : ne rend pas la section sale.
  untrackedInput.value = "sk-secret";
  untrackedInput.dispatchEvent("input");
  assert.equal(SettingsUI.isDirty("raccourcis"), false);

  // Changement radio : 2 inputs modifiés mais comptés pour 1 seule modification.
  radioA.checked = false;
  radioB.checked = true;
  radioB.dispatchEvent("change");
  assert.equal(SettingsUI.isDirty("raccourcis"), true);
  assert.equal(shortcutsApplyState.querySelector(".state-label").textContent, "1 modification non appliquée");
  assert.equal(linkShortcuts.classList.contains("dirty"), true);
  assert.equal(navDot.classList.contains("on"), true);

  // Seconde modification sur le champ texte : passe au pluriel et marque .field.changed.
  keyInput.value = "F8";
  keyInput.dispatchEvent("input");
  assert.equal(shortcutsField1.classList.contains("changed"), true);
  assert.equal(shortcutsApplyState.querySelector(".state-label").textContent, "2 modifications non appliquées");
});

test("SettingsUI isole les sections avec valueFor, pendingValues/restorePending et apply/revert", async () => {
  const { SettingsUI, keyInput, llmModel, panelShortcuts, shortcutsResult, llmResult } = setupSettingsUI();

  keyInput.value = "Ctrl+Space";
  keyInput.dispatchEvent("input");
  llmModel.value = "gpt-5-mini";
  llmModel.dispatchEvent("input");

  // Quand on applique « raccourcis », valueFor renvoie la saisie pour raccourcis et l'ancienne valeur pour nettoyage.
  assert.equal(SettingsUI.valueFor(keyInput, "raccourcis"), "Ctrl+Space");
  assert.equal(SettingsUI.valueFor(llmModel, "raccourcis"), "deepseek-chat");
  assert.equal(SettingsUI.valueFor(llmModel, "nettoyage"), "gpt-5-mini");

  // Sauvegarde et restauration des saisies en attente d'une autre section.
  const pending = SettingsUI.pendingValues(["nettoyage"]);
  llmModel.value = "deepseek-chat";
  SettingsUI.restorePending(pending);
  assert.equal(llmModel.value, "gpt-5-mini");

  // Application réussie de raccourcis : nettoie uniquement raccourcis, nettoyage reste sale.
  let appliedPanel = null;
  SettingsUI.define("raccourcis", {
    apply: async (name) => {
      appliedPanel = name;
      return "✓ Raccourcis appliqués";
    },
    revert: null,
  });
  panelShortcuts.requestSubmit();
  await new Promise((r) => setImmediate(r));
  assert.equal(appliedPanel, "raccourcis");
  assert.equal(SettingsUI.isDirty("raccourcis"), false);
  assert.equal(SettingsUI.isDirty("nettoyage"), true);
  assert.equal(shortcutsResult.textContent, "✓ Raccourcis appliqués");
  assert.equal(shortcutsResult.classList.contains("error"), false);

  // Annulation sur nettoyage : restaure la valeur d'origine du snapshot.
  SettingsUI.define("nettoyage", {
    apply: async () => {
      throw new Error("Serveur injoignable");
    },
    revert: null,
  });
  // Échec d'application : garde dirty et affiche l'erreur.
  const applyPromise = new Promise((resolve) => {
    const origShow = llmResult;
    setImmediate(() => resolve(origShow));
  });
  // Soumission qui échoue.
  const panelLlm = SettingsUI.panelsOfKind("llm");
  assert.deepEqual(panelLlm, ["nettoyage"]);
  const formLlm = llmModel.closest("form");
  formLlm.requestSubmit();
  await applyPromise;
  assert.equal(SettingsUI.isDirty("nettoyage"), true);
  assert.equal(llmResult.classList.contains("error"), true);
  assert.match(llmResult.textContent, /Serveur injoignable/);

  // Clic sur Annuler : remet la valeur d'origine.
  formLlm.querySelector(".revert").click();
  assert.equal(llmModel.value, "deepseek-chat");
  assert.equal(SettingsUI.isDirty("nettoyage"), false);
});

test("SettingsUI.show mémorise la dernière section et gère le repli par défaut", () => {
  const { env, SettingsUI, linkShortcuts, linkLlm, panelShortcuts, panelLlm } = setupSettingsUI();

  assert.equal(SettingsUI.show("nettoyage"), "nettoyage");
  assert.equal(env.localStorage.getItem("vtt-settings-panel"), "nettoyage");
  assert.equal(panelLlm.hidden, false);
  assert.equal(panelShortcuts.hidden, true);
  assert.equal(linkLlm.getAttribute("aria-current"), "true");
  assert.equal(linkShortcuts.getAttribute("aria-current"), null);

  // Section inconnue → reprend la dernière section mémorisée.
  assert.equal(SettingsUI.show("inconnue"), "nettoyage");

  // Sans historique en localStorage → section par défaut « raccourcis ».
  env.localStorage.clear();
  assert.equal(SettingsUI.show(""), "raccourcis");
  assert.equal(panelShortcuts.hidden, false);
});
