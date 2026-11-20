import test from "node:test";
import assert from "node:assert/strict";
import { createEnvironment } from "./dom.mjs";

const flush = () => new Promise((resolve) => setImmediate(resolve));

function setupMeetings(invoke) {
  const env = createEnvironment({ invoke });
  const { el, document } = env;

  const ids = [
    ["button", "conf-start"],
    ["button", "import-file"],
    ["input", "meeting-search"],
    ["button", "ask-send"],
    ["input", "ask-question"],
    ["button", "meeting-toggle"],
    ["input", "meeting-title"],
    ["button", "meeting-summarize"],
    ["button", "meeting-copy"],
    ["button", "meeting-export"],
    ["button", "meeting-audio-me"],
    ["button", "meeting-audio-them"],
    ["button", "meeting-retranscribe"],
    ["button", "meeting-identify"],
    ["select", "meeting-speaker-count"],
    ["button", "meeting-forget-speakers"],
    ["button", "meeting-delete"],
    ["button", "meeting-diarize-download"],
    ["input", "meeting-diarize"],
    ["p", "meeting-diarize-status"],
    ["div", "meeting-result"],
    ["article", "meeting-detail"],
    ["div", "meeting-speakers-block"],
    ["ul", "meeting-speakers"],
    ["p", "voicebook-empty"],
    ["ul", "voicebook-list"],
    ["button", "voicebook-clear"],
    ["div", "voicebook-result"],
  ];

  const root = el(
    "div",
    {},
    ...ids.map(([tag, id]) => el(tag, { id })),
  );
  // Sous-section Réglages → Réunions pour SettingsUI.define("reunions", …)
  const panel = el(
    "form",
    { className: "panel", attrs: { "data-panel": "reunions", "data-kind": "general" } },
    el("div", { className: "apply-state" }),
    el("div", { className: "panel-result", hidden: true }),
  );
  document.body.append(root, panel);
  env.context.option = (value, label) => el("option", { value, textContent: label });
  env.load("common.js", "settings-ui.js", "meetings.js");
  return { env, document };
}

test("renderSpeakers affiche le badge carnet, active le bouton Mémoriser au fil de la saisie et enrôle la voix", async () => {
  const calls = [];
  const { env, document } = setupMeetings(async (cmd, args) => {
    calls.push([cmd, args]);
    if (cmd === "enroll_speaker_voice") {
      return { id: "v-1", name: args.name.trim(), samples: 8, meetings: 1 };
    }
    if (cmd === "voicebook_view") return [];
    return null;
  });

  const detail = {
    record: { id: "1001", speaker_count: null, identifying: false },
    busy: false,
    voices_kept: true,
    speakers: [
      {
        index: 0,
        label: "Intervenant 1",
        name: "Marie",
        talk_ms: 42_000,
        first: "Bonjour à tous.",
        found: null,
        found_ms: null,
        recognized: { entry_id: "v-marie", name: "Marie", confidence: 94 },
      },
      {
        index: 1,
        label: "Intervenant 2",
        name: "",
        talk_ms: 15_000,
        first: "On commence ?",
        found: null,
        found_ms: null,
        recognized: null,
      },
    ],
  };

  env.context.renderSpeakers(detail);

  const items = document.getElementById("meeting-speakers").children;
  assert.equal(items.length, 2);

  // Intervenant 0 : reconnu dans le carnet, bouton « Enrichir la voix de Marie » actif.
  const recBadge = items[0].querySelector("span.recognized");
  assert.ok(recBadge);
  assert.match(recBadge.textContent, /94 %/);
  const enroll0 = items[0].querySelector(".speaker-enroll");
  assert.equal(enroll0.textContent, "Enrichir la voix de Marie");
  assert.equal(enroll0.disabled, false);

  // Intervenant 1 : sans nom → bouton désactivé, puis actif dès la saisie.
  const input1 = items[1].querySelector("input");
  const enroll1 = items[1].querySelector(".speaker-enroll");
  assert.equal(enroll1.textContent, "Mémoriser la voix");
  assert.equal(enroll1.disabled, true);

  input1.value = "Paul";
  input1.dispatchEvent({ type: "input" });
  assert.equal(enroll1.disabled, false);
  assert.equal(enroll1.textContent, "Mémoriser la voix de Paul");

  // Clic sur Mémoriser : appelle enroll_speaker_voice avec le nom saisi.
  enroll1.click();
  await flush();
  await flush();

  const enrollCall = calls.find(([cmd]) => cmd === "enroll_speaker_voice");
  assert.ok(enrollCall);
  assert.deepEqual(JSON.parse(JSON.stringify(enrollCall)), [
    "enroll_speaker_voice",
    { id: "1001", index: 1, name: "Paul" },
  ]);
  assert.match(document.getElementById("meeting-result").textContent, /Voix de Paul mémorisée/);

  // Sans empreintes conservées (audio en mémoire seulement) : aucun bouton de mémorisation.
  env.context.renderSpeakers({ ...detail, record: { ...detail.record, id: "1002" }, voices_kept: false });
  assert.equal(document.getElementById("meeting-speakers").querySelector(".speaker-enroll"), null);
});

test("showVoicebook gère l'affichage, le renommage, l'oubli en un clic et l'effacement en deux clics", async () => {
  let entries = [
    { id: "10", name: "Claire", samples: 12, meetings: 2 },
    { id: "20", name: "Paul", samples: 5, meetings: 1 },
  ];
  const calls = [];
  const { env, document } = setupMeetings(async (cmd, args) => {
    calls.push([cmd, args]);
    if (cmd === "voicebook_view") return entries.map((e) => ({ ...e }));
    if (cmd === "rename_voicebook_entry") {
      entries = entries.map((e) => (e.id === args.id ? { ...e, name: args.name } : e));
      return null;
    }
    if (cmd === "remove_voicebook_entry") {
      entries = entries.filter((e) => e.id !== args.id);
      return null;
    }
    if (cmd === "clear_voicebook") {
      entries = [];
      return null;
    }
    return null;
  });

  await env.context.showVoicebook();
  const list = document.getElementById("voicebook-list");
  assert.equal(document.getElementById("voicebook-empty").hidden, true);
  assert.equal(document.getElementById("voicebook-clear").hidden, false);
  assert.equal(list.children.length, 2);
  assert.equal(list.children[0].querySelector(".voicebook-meta").textContent, "2 réunions · 12 extraits");
  assert.equal(list.children[1].querySelector(".voicebook-meta").textContent, "1 réunion · 5 extraits");

  // Renommage de la première entrée.
  const nameInput = list.children[0].querySelector(".voicebook-name");
  nameInput.value = "Claire Martin";
  nameInput.dispatchEvent({ type: "change" });
  await flush();
  await flush();
  assert.ok(calls.some(([c, a]) => c === "rename_voicebook_entry" && a.id === "10" && a.name === "Claire Martin"));

  // Oubli en un clic de la seconde entrée.
  list.children[1].querySelector(".voicebook-remove").click();
  await flush();
  await flush();
  assert.ok(calls.some(([c, a]) => c === "remove_voicebook_entry" && a.id === "20"));
  assert.equal(list.children.length, 1);

  // Effacement complet : confirmation en deux clics.
  const clearBtn = document.getElementById("voicebook-clear");
  clearBtn.click();
  assert.equal(clearBtn.textContent, "Confirmer l'oubli");
  assert.equal(calls.filter(([c]) => c === "clear_voicebook").length, 0);

  clearBtn.click();
  await flush();
  await flush();
  assert.equal(calls.filter(([c]) => c === "clear_voicebook").length, 1);
  assert.equal(document.getElementById("voicebook-empty").hidden, false);
  assert.equal(document.getElementById("voicebook-clear").hidden, true);
});
