import test from "node:test";
import assert from "node:assert/strict";
import { createEnvironment } from "./dom.mjs";

const flush = () => new Promise((resolve) => setImmediate(resolve));

function setupMeetingsWithReport(invoke) {
  const env = createEnvironment({ invoke });
  const { el, document } = env;

  const ids = [
    ["button", "conf-start"],
    ["button", "import-file"],
    ["input", "meeting-search"],
    ["ol", "search-results"],
    ["p", "search-empty"],
    ["button", "ask-send"],
    ["input", "ask-question"],
    ["select", "ask-scope"],
    ["div", "ask-answer"],
    ["button", "meeting-toggle"],
    ["div", "meeting-live"],
    ["span", "meeting-elapsed"],
    ["ol", "meeting-list"],
    ["p", "meeting-empty"],
    ["input", "meeting-title"],
    ["p", "meeting-meta"],
    ["audio", "meeting-player"],
    ["p", "meeting-notes"],
    ["button", "meeting-summarize"],
    ["select", "meeting-summary-language"],
    ["button", "meeting-copy"],
    ["button", "meeting-export"],
    ["button", "meeting-audio-me"],
    ["button", "meeting-audio-them"],
    ["button", "meeting-retranscribe"],
    ["button", "meeting-identify"],
    ["select", "meeting-speaker-count"],
    ["button", "meeting-forget-speakers"],
    ["button", "meeting-delete"],
    ["div", "meeting-summary-block"],
    ["p", "meeting-summary-meta"],
    ["div", "meeting-summary"],
    ["p", "meeting-summary-error"],
    ["ol", "meeting-turns"],
    ["p", "meeting-turns-empty"],
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
    // Réglages Réunions remplis par fillMeetingSettings()
    ["input", "meeting-echo"],
    ["input", "meeting-detect"],
    ["select", "meeting-language"],
    ["input", "meeting-system"],
    ["input", "meeting-auto-summary"],
    ["select", "meeting-keep"],
    // Onglet Rapport complet
    ["div", "meeting-panel-transcript"],
    ["div", "meeting-panel-report"],
    ["button", "meeting-report-draft"],
    ["button", "meeting-report-copy"],
    ["button", "meeting-report-export"],
    ["p", "meeting-report-meta"],
    ["p", "meeting-report-error"],
    ["p", "meeting-report-empty"],
    ["iframe", "meeting-report-frame"],
  ];

  const tabTranscript = el("button", {
    id: "meeting-tab-btn-transcript",
    attrs: { "data-meeting-tab": "transcript", "aria-selected": "true", "aria-pressed": "true" },
  });
  const tabReport = el("button", {
    id: "meeting-tab-btn-report",
    attrs: { "data-meeting-tab": "report", "aria-selected": "false", "aria-pressed": "false" },
  });
  const providersTable = el("table", { id: "meeting-providers" }, el("tbody"));
  providersTable.tBodies = [providersTable.children[0]];

  const root = el(
    "div",
    {},
    tabTranscript,
    tabReport,
    providersTable,
    ...ids.map(([tag, id]) => el(tag, { id, hidden: id === "meeting-panel-report" })),
  );
  const panel = el(
    "form",
    { className: "panel", attrs: { "data-panel": "reunions", "data-kind": "general" } },
    el("div", { className: "apply-state" }),
    el("div", { className: "panel-result", hidden: true }),
  );
  document.body.append(root, panel);
  // Faux lecteur audio pour pause() / play().
  const player = document.getElementById("meeting-player");
  player.pause = () => {};
  player.play = async () => {};

  env.context.option = (value, label) => el("option", { value, textContent: label });
  env.context.window.matchMedia = () => ({ matches: false });
  env.load("common.js", "settings-ui.js", "meetings.js");
  return { env, document, tabTranscript, tabReport };
}

test("onglet Rapport complet charge le HTML interactif, branche window.VTT et coche une action", async () => {
  const calls = [];
  const sampleDetail = {
    record: {
      id: "m-1",
      title: "Comité produit",
      started_at_ms: 1_793_790_000_000,
      duration_ms: 120_000,
      status: "terminee",
      kind: "reunion",
      mic: true,
      system: true,
      notes: [],
      segments: [{ track: "moi", start_ms: 0, end_ms: 5000, text: "Budget validé." }],
      summary: null,
      summary_error: null,
      report: {
        at_ms: 1_793_790_120_000,
        model: "deepseek-chat",
        cost: 0.002,
        currency: "EUR",
        synthesis: "Point validé.",
        chapters: [{ title: "Budget", start_ms: 0, end_ms: 120_000 }],
        actions: [{ task: "Envoyer le devis", done: false }],
      },
      report_error: null,
    },
    turns: [{ track: "moi", speaker: null, who: "", start_ms: 0, text: "Budget validé." }],
    busy: false,
    audio_kept: false,
    replay: true,
    speakers: [],
    can_identify: false,
    voices_kept: false,
  };

  const { env, document, tabReport } = setupMeetingsWithReport(async (cmd, args) => {
    calls.push([cmd, args]);
    if (cmd === "meetings_view") {
      return {
        recording: null,
        elapsed_ms: 0,
        llm_enabled: true,
        meetings: [
          {
            id: "m-1",
            title: "Comité produit",
            started_at_ms: 1_793_790_000_000,
            duration_ms: 120_000,
            status: "terminee",
            segments: 1,
            has_summary: false,
            has_report: true,
            audio_kept: false,
            kind: "reunion",
            progress: null,
            identifying: false,
          },
        ],
        settings: {
          audio: "temporaire",
          system_audio: true,
          auto_summary: true,
          keep_days: 0,
          detect: true,
          echo_filter: true,
          summary_language: "",
          diarize: false,
        },
        keep_days: [0, 30],
        providers: [],
        languages: ["Français"],
      };
    }
    if (cmd === "meeting_detail") return sampleDetail;
    if (cmd === "meeting_report_html") return "<!doctype html><html><body>Rapport</body></html>";
    if (cmd === "voices_view") return { installed: false, downloading: false, error: null };
    if (cmd === "set_meeting_report_action_done") return null;
    if (cmd === "export_meeting_report_html") return "/tmp/comite-rapport.html";
    return null;
  });

  // Simule le chargement complet de la page Réunions.
  env.context.meetingSettingsFilled = true;
  await env.context.showMeetings();
  await flush();

  // Le panneau rapport est masqué au départ, le bouton indique « Refaire le rapport ».
  assert.equal(document.getElementById("meeting-panel-report").hidden, true);
  assert.equal(document.getElementById("meeting-report-draft").textContent, "Refaire le rapport");
  assert.equal(document.getElementById("meeting-report-copy").hidden, false);
  assert.equal(document.getElementById("meeting-report-export").hidden, false);

  // Bascule sur l'onglet Rapport complet → charge l'iframe avec le thème clair.
  tabReport.dispatchEvent({ type: "click" });
  await flush();

  assert.equal(document.getElementById("meeting-panel-report").hidden, false);
  assert.equal(document.getElementById("meeting-panel-transcript").hidden, true);
  assert.ok(calls.some(([c, a]) => c === "meeting_report_html" && a.id === "m-1" && a.theme === "light"));

  const frame = document.getElementById("meeting-report-frame");
  assert.equal(frame.hidden, false);
  assert.match(frame.srcdoc, /Rapport/);

  // Simule l'événement onload de l'iframe et vérifie le pont window.VTT.
  const searched = [];
  frame.contentWindow = {
    vttPlaying: () => {},
    vttSearch: (q) => searched.push(q),
  };
  frame.onload();
  assert.equal(typeof frame.contentWindow.VTT.setDone, "function");

  await frame.contentWindow.VTT.setDone(0, true);
  assert.ok(
    calls.some(
      ([c, a]) => c === "set_meeting_report_action_done" && a.id === "m-1" && a.index === 0 && a.done === true,
    ),
  );
  assert.match(document.getElementById("meeting-report-meta").textContent, /1 \/ 1 action/);

  // Export HTML.
  document.getElementById("meeting-report-export").dispatchEvent({ type: "click" });
  await flush();
  assert.ok(calls.some(([c, a]) => c === "export_meeting_report_html" && a.id === "m-1"));
});

test("recherche globale sur un résultat de rapport bascule vers l'onglet Rapport et lance vttSearch", async () => {
  const searched = [];
  const { env, document } = setupMeetingsWithReport(async (cmd) => {
    if (cmd === "meetings_view") {
      return {
        recording: null,
        elapsed_ms: 0,
        llm_enabled: true,
        meetings: [
          {
            id: "m-2",
            title: "Revue budget",
            started_at_ms: 1_793_790_000_000,
            duration_ms: 180_000,
            status: "terminee",
            segments: 1,
            has_summary: false,
            has_report: true,
            audio_kept: false,
            kind: "reunion",
            progress: null,
            identifying: false,
          },
        ],
        settings: {
          audio: "temporaire",
          system_audio: true,
          auto_summary: true,
          keep_days: 0,
          detect: true,
          echo_filter: true,
          summary_language: "",
          diarize: false,
        },
        keep_days: [0, 30],
        providers: [],
        languages: ["Français"],
      };
    }
    if (cmd === "search_meetings") {
      return [
        {
          id: "m-2",
          title: "Revue budget",
          kind: "reunion",
          started_at_ms: 1_793_790_000_000,
          start_ms: 90_000,
          parts: [
            ["Validation du ", false],
            ["budget", true],
          ],
          replay: false,
          source: "report",
          section: "Décision",
        },
      ];
    }
    if (cmd === "meeting_detail") {
      return {
        record: {
          id: "m-2",
          title: "Revue budget",
          started_at_ms: 1_793_790_000_000,
          duration_ms: 180_000,
          status: "terminee",
          kind: "reunion",
          mic: true,
          system: true,
          notes: [],
          segments: [{ track: "moi", start_ms: 0, end_ms: 5000, text: "Test." }],
          report: { at_ms: 1_793_790_200_000, model: "m", chapters: [], actions: [] },
        },
        turns: [],
        busy: false,
        audio_kept: false,
        replay: false,
        speakers: [],
        can_identify: false,
        voices_kept: false,
      };
    }
    if (cmd === "meeting_report_html") return "<!doctype html><html><body>Rapport 2</body></html>";
    if (cmd === "voices_view") return { installed: false, downloading: false, error: null };
    return null;
  });

  await env.context.showMeetings();
  await flush();

  document.getElementById("meeting-search").value = "budget";
  await env.context.runSearch();

  const hitBtn = document.getElementById("search-results").children[0].children[0];
  assert.match(hitBtn.children[0].textContent, /Rapport \(Décision\) · 01:30/);

  await hitBtn.dispatchEvent({ type: "click" });
  await flush();

  assert.equal(document.getElementById("meeting-panel-report").hidden, false);
  const frame = document.getElementById("meeting-report-frame");
  frame.contentWindow = { vttSearch: (q) => searched.push(q) };
  frame.onload();
  assert.deepEqual(searched, ["budget"]);
});
