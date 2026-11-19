import test from "node:test";
import assert from "node:assert/strict";
import { createEnvironment } from "./dom.mjs";

async function setupOverlay(initial = {}) {
  const env = createEnvironment({
    now: 1_700_000_000_000,
    invoke: async (cmd) => {
      if (cmd === "current_status") return initial.status ?? { state: "idle" };
      if (cmd === "meeting_state") return initial.meeting ?? { recording: false, elapsed_ms: 0 };
      if (cmd === "annotate_state") return initial.annotate ?? { recording: false, elapsed_ms: 0, drawing: false };
      if (cmd === "translation_state") return initial.translation ?? null;
      return {};
    },
  });
  const { el, document } = env;

  const pill = el("div", { id: "pill", attrs: { "data-state": "idle" } });
  const label = el("span", { id: "label" });
  const lang = el("span", { id: "lang", hidden: true });
  const preview = el("div", { id: "preview", hidden: true });
  const meter = el(
    "div",
    { id: "meter" },
    el("i"),
    el("i"),
    el("i"),
    el("i"),
    el("i"),
  );
  document.body.append(pill, label, lang, preview, meter);
  env.load("overlay.js");
  await new Promise((r) => setImmediate(r));
  return { env, pill, label, lang, preview, bars: meter.children };
}

test("overlay formate elapsed et gère les transitions d'état et de traduction", async () => {
  const { env, pill, label, lang } = await setupOverlay({ translation: "anglais" });
  const elapsed = env.eval("elapsed");

  assert.equal(elapsed(1_700_000_000_000 - 5_000), "00:05");
  assert.equal(elapsed(1_700_000_000_000 - 754_000), "12:34");
  assert.equal(elapsed(1_700_000_000_000 - 3_723_000), "1:02:03");

  // Au repos : badge langue caché.
  assert.equal(pill.dataset.state, "idle");
  assert.equal(lang.hidden, true);

  // En écoute : affiche le libellé et le code langue « → EN ».
  env.emitTauri("status", { state: "listening" });
  assert.equal(pill.dataset.state, "listening");
  assert.equal(label.textContent, "Écoute…");
  assert.equal(lang.hidden, false);
  assert.equal(lang.textContent, "→ EN");

  // En transcription : badge langue caché.
  env.emitTauri("status", { state: "processing" });
  assert.equal(label.textContent, "Transcription…");
  assert.equal(lang.hidden, true);

  // Mains libres : libellé court prioritaire sur le message complet.
  env.emitTauri("status", { state: "handsfree", message: "AltGr mains libres" });
  assert.equal(label.textContent, "Mains libres · Échap annule");
  assert.equal(lang.hidden, false);

  // Erreur : revient au repos après 5 s.
  env.emitTauri("status", { state: "error", message: "Micro occupé" });
  assert.equal(pill.dataset.state, "error");
  assert.equal(label.textContent, "Micro occupé");
  env.advanceTime(5000);
  assert.equal(pill.dataset.state, "idle");
});

test("overlay affiche réunion, capture commentée, barres de niveau et visibilité différée", async () => {
  const { env, pill, label, bars } = await setupOverlay();

  // Réunion démarrée avec rappel de consentement : flash 5 s puis chrono « Conférence 01:05 ».
  env.emitTauri("meeting", { recording: true, conference: true, elapsed_ms: 60_000, consent: true });
  assert.equal(pill.dataset.state, "info");
  assert.equal(label.textContent, "Réunion : prévenez les participants");
  env.advanceTime(5000);
  assert.equal(pill.dataset.state, "meeting");
  assert.equal(label.textContent, "Conférence 01:05");

  // Capture commentée prend la priorité sur la réunion et indique le dessin.
  env.emitTauri("annotate", { recording: true, elapsed_ms: 10_000, drawing: true });
  assert.equal(pill.dataset.state, "meeting");
  assert.equal(label.textContent, "Dessin…");

  // Arrêt capture + réunion : retour au repos et masquage différé (200 ms) envoyé à Tauri.
  env.emitTauri("annotate", { recording: false, elapsed_ms: 0, drawing: false });
  env.emitTauri("meeting", { recording: false, elapsed_ms: 0, consent: false });
  assert.equal(pill.dataset.state, "idle");
  const visibleBefore = env.invokeCalls.filter((c) => c.cmd === "overlay_visible");
  assert.equal(visibleBefore[visibleBefore.length - 1].args.visible, true);
  env.advanceTime(200);
  const visibleAfter = env.invokeCalls.filter((c) => c.cmd === "overlay_visible");
  assert.equal(visibleAfter[visibleAfter.length - 1].args.visible, false);

  // Niveau micro borné entre 0 et 1, avec plancher visuel 0.2.
  env.emitTauri("level", 2.5);
  assert.equal(bars[2].style.transform, "scaleY(1)");
  assert.equal(bars[0].style.transform, "scaleY(0.55)");
  env.emitTauri("level", -1);
  assert.equal(bars[2].style.transform, "scaleY(0.2)");
});

test("overlay affiche l'aperçu glissant uniquement en mode mains libres et l'efface à la sortie", async () => {
  const { env, pill, preview } = await setupOverlay();

  // Hors mains libres : ignoré.
  env.emitTauri("status", { state: "listening" });
  env.emitTauri("live-preview", "Ne doit pas s'afficher");
  assert.equal(preview.hidden, true);
  assert.equal(preview.textContent, "");
  assert.equal(pill.classList.contains("has-preview"), false);

  // En mains libres : affiché et classe has-preview activée.
  env.emitTauri("status", { state: "handsfree", message: "AltGr" });
  env.emitTauri("live-preview", "… ceci est une dictée longue en direct.");
  assert.equal(preview.hidden, false);
  assert.equal(preview.textContent, "… ceci est une dictée longue en direct.");
  assert.equal(pill.classList.contains("has-preview"), true);

  // Passage en transcription finale : aperçu effacé proprement.
  env.emitTauri("status", { state: "processing" });
  assert.equal(preview.hidden, true);
  assert.equal(preview.textContent, "");
  assert.equal(pill.classList.contains("has-preview"), false);

  // Retour en mains libres puis effacement explicite par chaîne vide.
  env.emitTauri("status", { state: "handsfree", message: "AltGr" });
  env.emitTauri("live-preview", "Nouvelle phrase");
  assert.equal(preview.hidden, false);
  env.emitTauri("live-preview", "   ");
  assert.equal(preview.hidden, true);
  assert.equal(pill.classList.contains("has-preview"), false);
});
