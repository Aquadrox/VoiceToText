import test from "node:test";
import assert from "node:assert/strict";
import { createEnvironment } from "./dom.mjs";

function setupHistory(invoke) {
  const env = createEnvironment({ invoke });
  const { el, document } = env;

  const tplRoot = el(
    "li",
    { className: "entry" },
    el("span", { className: "when" }),
    el("span", { className: "badge" }),
    el("p", { className: "text" }),
    el("details", {}, el("summary", { textContent: "Brut" }), el("p", { className: "raw" })),
    el("button", { className: "copy", textContent: "Copier" }),
  );
  const tpl = document.createTemplate(tplRoot);
  tpl.id = "history-entry";

  const disabledBanner = el("div", { id: "history-disabled", hidden: true });
  const clearBtn = el("button", { id: "history-clear", textContent: "Tout effacer" });
  const search = el("input", { id: "history-search", value: "" });
  const list = el("ul", { id: "history-list" });
  const empty = el("p", { id: "history-empty", hidden: true });

  document.body.append(tpl, disabledBanner, clearBtn, search, list, empty);
  env.load("common.js", "history.js");

  return { env, disabledBanner, clearBtn, search, list, empty };
}

test("showHistory affiche les entrées, badges prioritaires, détails bruts et filtre de recherche", async () => {
  const entries = [
    {
      at_ms: 1_700_000_000_000,
      duration_ms: 2400,
      app: "code.exe",
      cost: 0.00025,
      currency: "USD",
      usage: { input_tokens: 120, output_tokens: 45 },
      command: true,
      text: "Fonction refactorisée",
      raw: "refactorise cette fonction",
    },
    {
      at_ms: 1_700_000_010_000,
      duration_ms: 1000,
      snippet: true,
      text: "Cordialement,",
      raw: "Cordialement,",
    },
    {
      at_ms: 1_700_000_020_000,
      duration_ms: 3000,
      translated: "anglais",
      text: "Hello world",
      raw: "Bonjour le monde",
    },
    {
      at_ms: 1_700_000_030_000,
      duration_ms: 1500,
      fallback: "Ollama",
      text: "Texte secours",
      raw: "texte secours",
    },
  ];

  const { env, disabledBanner, clearBtn, search, list, empty } = setupHistory(async (cmd) => {
    if (cmd === "history_view") return { enabled: true, entries };
    return {};
  });

  await env.eval("showHistory")();
  assert.equal(disabledBanner.hidden, true);
  assert.equal(clearBtn.disabled, false);
  assert.equal(list.children.length, 4);
  assert.equal(empty.hidden, true);

  // Entrée commande : badge, summary "Consigne", tooltip jetons, coût.
  const first = list.children[0];
  assert.equal(first.querySelector(".badge").textContent, "commande");
  assert.equal(first.querySelector("details summary").textContent, "Consigne");
  assert.equal(first.querySelector(".raw").textContent, "refactorise cette fonction");
  assert.match(first.querySelector(".when").title, /120 jetons envoyés, 45 reçus/);
  assert.match(first.querySelector(".when").textContent, /code\.exe/);

  // Entrée snippet avec raw === text : le bloc <details> est supprimé.
  assert.equal(list.children[1].querySelector(".badge").textContent, "snippet");
  assert.equal(list.children[1].querySelector("details"), null);

  // Badges traduction et secours.
  assert.equal(list.children[2].querySelector(".badge").textContent, "traduit : anglais");
  assert.equal(list.children[3].querySelector(".badge").textContent, "secours : Ollama");

  // Recherche insensible à la casse dans le texte brut (`raw`).
  search.value = "BONJOUR";
  search.dispatchEvent("input");
  assert.equal(list.children.length, 1);
  assert.equal(list.children[0].querySelector(".text").textContent, "Hello world");

  // Aucun résultat.
  search.value = "introuvable";
  search.dispatchEvent("input");
  assert.equal(list.children.length, 0);
  assert.equal(empty.hidden, false);
  assert.equal(empty.textContent, "Aucun résultat.");
});

test("history-clear demande une confirmation en deux clics et la copie met à jour le bouton", async () => {
  let cleared = false;
  const { env, clearBtn, list, empty } = setupHistory(async (cmd) => {
    if (cmd === "clear_history") {
      cleared = true;
      return {};
    }
    return {
      enabled: true,
      entries: cleared
        ? []
        : [{ at_ms: 1_700_000_000_000, duration_ms: 1000, text: "Salut", raw: "Salut" }],
    };
  });

  await env.eval("showHistory")();

  // Copie dans le presse-papier puis retour au libellé d'origine après 1,5 s.
  const copyBtn = list.children[0].querySelector(".copy");
  copyBtn.click();
  await new Promise((r) => setImmediate(r));
  assert.deepEqual(env.clipboard.written, ["Salut"]);
  assert.equal(copyBtn.textContent, "Copié ✓");
  env.advanceTime(1500);
  assert.equal(copyBtn.textContent, "Copier");

  // Premier clic : arme la confirmation sans effacer ; expiration au bout de 4 s.
  clearBtn.click();
  assert.equal(clearBtn.textContent, "Confirmer l'effacement");
  assert.equal(cleared, false);
  env.advanceTime(4000);
  assert.equal(clearBtn.textContent, "Tout effacer");

  // Double clic dans les 4 s : efface réellement et recharge la liste vide.
  clearBtn.click();
  clearBtn.click();
  await new Promise((r) => setImmediate(r));
  assert.equal(cleared, true);
  assert.equal(clearBtn.disabled, true);
  assert.equal(empty.hidden, false);
  assert.equal(empty.textContent, "Aucune dictée pour l'instant.");
});
