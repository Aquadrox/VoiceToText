import test from "node:test";
import assert from "node:assert/strict";
import { createEnvironment } from "./dom.mjs";

function setupVocabulary(invoke) {
  const env = createEnvironment({ invoke });
  const { el, document } = env;

  const dictForm = el("form", { id: "dict-form" });
  const dictInput = el("input", { id: "dict-input", value: "" });
  const dictList = el("ul", { id: "dict-list" });
  const dictEmpty = el("p", { id: "dict-empty", hidden: true });
  const dictSave = el("button", { id: "dict-save" });
  const dictStatus = el("p", { id: "dict-status" });
  const repForm = el("form", { id: "rep-form" });
  const repHeard = el("input", { id: "rep-heard", value: "" });
  const repWritten = el("input", { id: "rep-written", value: "" });
  const repList = el("ul", { id: "rep-list" });
  const repEmpty = el("p", { id: "rep-empty", hidden: true });
  const repSave = el("button", { id: "rep-save" });
  const repStatus = el("p", { id: "rep-status" });
  const suggestionList = el("ul", { id: "suggestion-list" });
  const suggestionEmpty = el("p", { id: "suggestion-empty", hidden: true });

  const snippetTpl = document.createTemplate(
    el(
      "div",
      { className: "card" },
      el("input", { className: "trigger" }),
      el("textarea", { className: "text" }),
      el("button", { className: "remove" }),
    ),
  );
  snippetTpl.id = "snippet-card";
  const snippetList = el("div", { id: "snippet-list" });
  const snippetAdd = el("button", { id: "snippet-add" });
  const snippetSave = el("button", { id: "snippet-save" });
  const snippetStatus = el("p", { id: "snippet-status" });

  const styleTpl = document.createTemplate(
    el(
      "div",
      { className: "card" },
      el("span", { className: "style-name" }),
      el("input", { className: "apps" }),
      el("button", { className: "detect-app", textContent: "Détecter" }),
      el("input", { className: "titles" }),
      el("input", { className: "cleanup", type: "checkbox" }),
      el("textarea", { className: "instructions" }),
      el("button", { className: "up" }),
      el("button", { className: "remove" }),
    ),
  );
  styleTpl.id = "style-card";
  const styleList = el("div", { id: "style-list" });
  const styleAdd = el("button", { id: "style-add" });
  const styleSave = el("button", { id: "style-save" });
  const styleStatus = el("p", { id: "style-status" });
  const tplCode = el("button", { attrs: { "data-template": "code" } });
  const tplEmail = el("button", { attrs: { "data-template": "email" } });

  document.body.append(
    dictForm,
    dictInput,
    dictList,
    dictEmpty,
    dictSave,
    dictStatus,
    repForm,
    repHeard,
    repWritten,
    repList,
    repEmpty,
    repSave,
    repStatus,
    suggestionList,
    suggestionEmpty,
    snippetTpl,
    snippetList,
    snippetAdd,
    snippetSave,
    snippetStatus,
    styleTpl,
    styleList,
    styleAdd,
    styleSave,
    styleStatus,
    tplCode,
    tplEmail,
  );

  env.load("common.js", "vocabulary.js");
  return {
    env,
    dictForm,
    dictInput,
    dictList,
    dictEmpty,
    dictSave,
    dictStatus,
    repForm,
    repHeard,
    repWritten,
    repList,
    repEmpty,
    repSave,
    repStatus,
    suggestionList,
    styleList,
    tplCode,
    tplEmail,
  };
}

test("vocabulary dédoublonne le dictionnaire sans casse et filtre les suggestions déjà connues", async () => {
  const { env, dictForm, dictInput, dictList, dictStatus, suggestionList } = setupVocabulary(async (cmd) => {
    if (cmd === "vocabulary_view") return { dictionary: ["Tauri"], replacements: [], snippets: [], styles: [] };
    if (cmd === "dictionary_suggestions") {
      return {
        history_enabled: true,
        suggestions: [
          { word: "tauri", count: 3, corrected: true },
          { word: "Parakeet", count: 2, corrected: false },
        ],
      };
    }
    return {};
  });

  assert.deepEqual(Array.from(env.eval("splitList")(" code.exe, , cursor.exe , ")), ["code.exe", "cursor.exe"]);

  await env.eval("PAGES.dictionnaire.show")();
  await new Promise((r) => setImmediate(r));

  // « tauri » est déjà dans le dictionnaire : seule « Parakeet » reste proposée.
  assert.equal(dictList.children.length, 1);
  assert.equal(suggestionList.children.length, 1);
  assert.equal(suggestionList.children[0].querySelector("span").textContent, "Parakeet");

  // Ajout d'un doublon en minuscules : ignoré, section pas marquée sale.
  dictInput.value = "  TAURI  ";
  dictForm.requestSubmit();
  assert.equal(dictList.children.length, 1);
  assert.equal(env.eval("dirty.dictionary"), false);

  // Ajout d'un nouveau mot : ajouté, champ vidé, section marquée sale.
  dictInput.value = "WeSpeaker";
  dictForm.requestSubmit();
  assert.equal(dictList.children.length, 2);
  assert.equal(dictInput.value, "");
  assert.equal(env.eval("dirty.dictionary"), true);
  assert.equal(dictStatus.textContent, "Modifications non enregistrées");

  // Recharger ne doit pas écraser une section modifiée non enregistrée.
  await env.eval("loadVocabulary")();
  assert.deepEqual(Array.from(env.eval("vocabulary.dictionary")), ["Tauri", "WeSpeaker"]);
});

test("corrections « entendu → écrit » : ajout, mise à jour sans doublon, suppression et sauvegarde", async () => {
  let saved = null;
  const { env, repForm, repHeard, repWritten, repList, repEmpty, repSave, repStatus } = setupVocabulary(
    async (cmd, args) => {
      if (cmd === "vocabulary_view") {
        return {
          dictionary: [],
          replacements: saved || [{ heard: "poste graisse", written: "Postgres" }],
          snippets: [],
          styles: [],
        };
      }
      if (cmd === "save_vocabulary") {
        saved = args.update.replacements.map((r) => ({ ...r }));
        return {};
      }
      if (cmd === "dictionary_suggestions") {
        return { history_enabled: true, suggestions: [] };
      }
      return {};
    },
  );

  await env.eval("PAGES.dictionnaire.show")();
  await new Promise((r) => setImmediate(r));
  assert.equal(repList.children.length, 1);
  assert.equal(repEmpty.hidden, true);

  // Même « entendu » avec casse/ponctuation différente : met à jour la règle existante.
  repHeard.value = "Poste graisse !";
  repWritten.value = "PostgreSQL";
  repForm.requestSubmit();
  assert.equal(repList.children.length, 1);
  assert.equal(env.eval("dirty.replacements"), true);

  // Nouvelle règle : ajoutée.
  repHeard.value = "aquatroc";
  repWritten.value = "Aquadrox";
  repForm.requestSubmit();
  assert.equal(repList.children.length, 2);

  // Sauvegarde : envoie uniquement la section `replacements`.
  repSave.click();
  await new Promise((r) => setImmediate(r));
  await new Promise((r) => setImmediate(r));
  assert.equal(env.eval("dirty.replacements"), false);
  assert.match(repStatus.textContent, /Enregistré/);
  assert.deepEqual(saved, [
    { heard: "Poste graisse !", written: "PostgreSQL" },
    { heard: "aquatroc", written: "Aquadrox" },
  ]);

  // Suppression d'une règle.
  repList.children[0].querySelector("button").click();
  assert.equal(repList.children.length, 1);
  assert.equal(env.eval("dirty.replacements"), true);
});

test("styles clone les modèles sans les altérer, gère cleanup et réordonne la priorité", () => {
  const { env, styleList, tplCode, tplEmail } = setupVocabulary(async () => ({
    dictionary: [],
    replacements: [],
    snippets: [],
    styles: [],
  }));

  tplCode.click();
  tplEmail.click();
  assert.equal(styleList.children.length, 2);

  // Modèle « code » : cleanup=false → instructions désactivées.
  const firstCard = styleList.children[0];
  const secondCard = styleList.children[1];
  assert.equal(firstCard.querySelector(".cleanup").checked, false);
  assert.equal(firstCard.querySelector(".instructions").disabled, true);
  assert.equal(firstCard.querySelector(".up").disabled, true);
  assert.equal(secondCard.querySelector(".up").disabled, false);

  // Modifier les apps du style cloné ne modifie pas STYLE_TEMPLATES.code.
  const appsInput = firstCard.querySelector(".apps");
  appsInput.value = "nvim.exe";
  appsInput.dispatchEvent("input");
  assert.ok(env.eval("STYLE_TEMPLATES.code.apps").includes("code.exe"));

  // Remonter le 2e style (email) en 1re position.
  secondCard.querySelector(".up").click();
  assert.equal(env.eval("vocabulary.styles")[0].cleanup, true);
  assert.equal(env.eval("vocabulary.styles")[1].cleanup, false);
});

test("styles détecte une application par clic et évite les doublons", async () => {
  let calls = 0;
  const { env, styleList, tplCode } = setupVocabulary(async (cmd) => {
    if (cmd === "capture_window_app") {
      calls += 1;
      return calls === 1 ? "banque.exe" : "Banque.EXE";
    }
    return { dictionary: [], replacements: [], snippets: [], styles: [] };
  });

  tplCode.click();
  const card = styleList.children[0];
  const detect = card.querySelector(".detect-app");
  const appsInput = card.querySelector(".apps");

  detect.click();
  await new Promise((r) => setImmediate(r));
  assert.ok(appsInput.value.includes("banque.exe"));
  assert.equal(detect.disabled, false);
  assert.equal(detect.textContent, "Détecter");

  const before = appsInput.value;
  detect.click();
  await new Promise((r) => setImmediate(r));
  assert.equal(appsInput.value, before);
});
