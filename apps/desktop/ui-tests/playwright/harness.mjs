import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright-core";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
export const UI_DIR = path.resolve(__dirname, "../../ui");

const MIME_TYPES = {
  ".html": "text/html; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".js": "application/javascript; charset=utf-8",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".ico": "image/x-icon",
  ".json": "application/json; charset=utf-8",
  ".txt": "text/plain; charset=utf-8",
};

/**
 * Trouve un navigateur Chromium déjà présent (Chrome préinstallé sur
 * ubuntu-latest ou variable d'environnement), sinon laisse Playwright utiliser
 * son propre binaire.
 */
export function resolveChromePath() {
  const candidates = [
    process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH,
    "/usr/bin/google-chrome",
    "/usr/bin/google-chrome-stable",
    "/usr/bin/chromium",
    "/usr/bin/chromium-browser",
  ].filter(Boolean);
  for (const candidate of candidates) {
    if (fs.existsSync(candidate)) return candidate;
  }
  return undefined;
}

export async function launchBrowser() {
  const executablePath = resolveChromePath();
  return chromium.launch({
    headless: true,
    ...(executablePath ? { executablePath } : {}),
    args: ["--no-sandbox", "--disable-dev-shm-usage", "--disable-gpu"],
  });
}

/** Démarre un serveur HTTP statique local sur 127.0.0.1:0 pour `apps/desktop/ui`. */
export async function startUiServer() {
  const server = http.createServer((req, res) => {
    const rawPath = decodeURIComponent((req.url || "/").split("?")[0]);
    if (rawPath === "/favicon.ico") {
      res.writeHead(204);
      res.end();
      return;
    }
    const cleanPath = rawPath === "/" ? "/app.html" : rawPath;
    const filePath = path.resolve(UI_DIR, `.${cleanPath}`);
    if (!filePath.startsWith(UI_DIR) || !fs.existsSync(filePath) || fs.statSync(filePath).isDirectory()) {
      res.writeHead(404);
      res.end("Not found");
      return;
    }
    const ext = path.extname(filePath).toLowerCase();
    res.writeHead(200, { "Content-Type": MIME_TYPES[ext] || "application/octet-stream" });
    fs.createReadStream(filePath).pipe(res);
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const { port } = server.address();
  return {
    baseUrl: `http://127.0.0.1:${port}`,
    close: () => new Promise((resolve) => server.close(resolve)),
  };
}

/** Document HTML interactif réaliste (même contrat `window.VTT` / `vttSearch` que `report.rs`). */
export const SAMPLE_REPORT_HTML = `<!doctype html>
<html lang="fr" data-mode="app" data-theme="light">
<head>
  <meta charset="utf-8" />
  <title>Rapport — Comité produit</title>
  <style>
    body { font-family: system-ui, sans-serif; margin: 16px; color: #1b2430; background: #faf9f6; }
    .action.done label { text-decoration: line-through; opacity: .65; }
    mark.hit { background: #ffe58f; padding: 0 2px; }
  </style>
</head>
<body>
  <header>
    <input id="q" type="search" placeholder="Rechercher dans le rapport…" />
    <span id="count"></span>
  </header>
  <main id="doc">
    <section id="synthese" data-section="synthese">
      <h2>Synthèse</h2>
      <p>Validation du budget marketing pour la rentrée.</p>
    </section>
    <section id="plan-action" data-section="actions">
      <h2>Plan d'action</h2>
      <span id="done-count" data-total="1">0 / 1 faite(s)</span>
      <ol class="actions">
        <li class="action item k-action">
          <label><input type="checkbox" data-index="0" /> Envoyer le devis final (Julie)</label>
        </li>
      </ol>
    </section>
  </main>
  <script>
    (function () {
      var doc = document.getElementById("doc");
      var input = document.getElementById("q");
      var count = document.getElementById("count");
      var doneCount = document.getElementById("done-count");
      doc.addEventListener("change", function (e) {
        if (!e.target.matches(".action input[type=checkbox]")) return;
        e.target.closest(".action").classList.toggle("done", e.target.checked);
        if (window.VTT) window.VTT.setDone(Number(e.target.dataset.index), e.target.checked);
        var done = doc.querySelectorAll(".action input:checked").length;
        doneCount.textContent = done + " / " + doneCount.dataset.total + " faite(s)";
      });
      window.vttSearch = function (query) {
        input.value = query || "";
        var p = doc.querySelector("#synthese p");
        if (!query) return;
        var text = p.textContent;
        var idx = text.toLowerCase().indexOf(query.toLowerCase());
        if (idx >= 0) {
          p.textContent = "";
          p.append(
            document.createTextNode(text.slice(0, idx)),
            Object.assign(document.createElement("mark"), { className: "hit", textContent: text.slice(idx, idx + query.length) }),
            document.createTextNode(text.slice(idx + query.length)),
          );
          count.textContent = "1 / 1";
        }
      };
      window.vttPlaying = function () {};
    })();
  </script>
</body>
</html>`;

/** Jeu de réponses Tauri réalistes pour parcourir toutes les vues sans backend natif. */
export function defaultMockData() {
  const now = 1_795_000_000_000;
  return {
    home_view: {
      summary: {
        total: { words: 12450, dictations: 420, audio_ms: 5_650_000, cost: 1.8 },
        month: { words: 3200, dictations: 90, audio_ms: 1_450_000, cost: 0.42 },
        today: { words: 510, dictations: 12, audio_ms: 230_000, cost: 0.02 },
      },
      currency: "EUR",
      days: [
        { day: "2026-11-23", totals: { words: 420, dictations: 10, audio_ms: 190_000, cost: 0.01 } },
        { day: "2026-11-24", totals: { words: 680, dictations: 15, audio_ms: 300_000, cost: 0.03 } },
        { day: "2026-11-25", totals: { words: 510, dictations: 12, audio_ms: 230_000, cost: 0.02 } },
      ],
      apps: {
        month: [{ app: "firefox", totals: { words: 1800, dictations: 65, cost: 0.21 } }],
        total: [{ app: "firefox", totals: { words: 7200, dictations: 210, cost: 0.85 } }],
      },
      update_news: null,
    },
    history_view: [
      {
        at_ms: now,
        raw: "euh bonjour a tous",
        text: "Bonjour à tous.",
        app: "firefox",
        mode: "dictee",
        cost: 0.0004,
        currency: "EUR",
        usage: { input_tokens: 120, cached_input_tokens: 80, output_tokens: 15 },
      },
    ],
    meetings_view: {
      recording: null,
      elapsed_ms: 0,
      llm_enabled: true,
      meetings: [
        {
          id: "m-1",
          title: "Comité produit",
          started_at_ms: now,
          duration_ms: 300_000,
          status: "terminee",
          segments: 2,
          has_summary: true,
          has_report: true,
          audio_kept: true,
          kind: "reunion",
          progress: null,
          identifying: false,
        },
      ],
      settings: {
        audio: "conserve",
        system_audio: true,
        auto_summary: true,
        keep_days: 30,
        detect: true,
        auto_stop: true,
        auto_stop_delay_sec: 15,
        auto_stop_silence_min: 0,
        echo_filter: true,
        summary_language: "",
        diarize: true,
        providers: [],
      },
      keep_days: [0, 7, 30, 90],
      auto_stop_delays: [10, 15, 30, 60],
      auto_stop_silence_mins: [0, 2, 5, 10, 15],
      providers: [
        { key: "teams", label: "Teams", browser: false, policy: "demander" },
        { key: "meet", label: "Google Meet", browser: true, policy: "demander" },
      ],
      languages: ["Français", "Anglais"],
    },
    meeting_detail: {
      record: {
        id: "m-1",
        title: "Comité produit",
        started_at_ms: now,
        duration_ms: 300_000,
        status: "terminee",
        kind: "reunion",
        mic: true,
        system: true,
        notes: [],
        segments: [
          { track: "moi", start_ms: 0, end_ms: 5000, text: "Bonjour à tous." },
          { track: "eux", start_ms: 6000, end_ms: 14000, text: "Le budget est validé." },
        ],
        summary: {
          text: "## Résumé\nValidation du budget marketing.",
          at_ms: now + 310_000,
          model: "deepseek-chat",
          cost: 0.0012,
          currency: "EUR",
        },
        summary_error: null,
        report: {
          at_ms: now + 320_000,
          model: "deepseek-chat",
          cost: 0.0025,
          currency: "EUR",
          synthesis: "Validation du budget marketing pour la rentrée.",
          chapters: [{ title: "Budget", start_ms: 0, end_ms: 300_000 }],
          actions: [{ task: "Envoyer le devis final", owner: "Julie", due: "vendredi", done: false }],
        },
        report_error: null,
        speaker_count: null,
        identifying: false,
      },
      turns: [
        { track: "moi", speaker: null, who: "", start_ms: 0, text: "Bonjour à tous." },
        { track: "eux", speaker: 0, who: "Marie", start_ms: 6000, text: "Le budget est validé." },
      ],
      busy: false,
      audio_kept: true,
      replay: true,
      speakers: [
        {
          index: 0,
          label: "Intervenant 1",
          name: "Marie",
          talk_ms: 48_000,
          first: "Le budget est validé.",
          found: null,
          found_ms: null,
          recognized: { entry_id: "v-marie", name: "Marie", confidence: 94 },
        },
      ],
      can_identify: false,
      voices_kept: true,
    },
    meeting_report_html: SAMPLE_REPORT_HTML,
    search_meetings: [
      {
        id: "m-1",
        title: "Comité produit",
        kind: "reunion",
        started_at_ms: now,
        start_ms: 6000,
        parts: [
          ["Validation du ", false],
          ["budget", true],
          [" marketing pour la rentrée.", false],
        ],
        replay: true,
        source: "report",
        section: "Synthèse",
        match_kind: "hybrid",
        semantic_score: 0.86,
      },
      {
        id: "m-1",
        title: "Comité produit",
        kind: "reunion",
        started_at_ms: now,
        start_ms: 12000,
        parts: [["Enveloppe financière approuvée pour la campagne.", false]],
        replay: true,
        source: "transcript",
        section: null,
        match_kind: "semantic",
        semantic_score: 0.82,
      },
    ],
    ask_meetings: {
      text: "Le budget marketing est validé à 5 000 € [Comité produit · 00:06].",
      cost: 0.0004,
      currency: "€",
      sources: 1,
      semantic: true,
    },
    voices_view: { installed: true, downloading: false, progress: 100, error: null },
    semantic_view: {
      installed: true,
      missing_mb: 0,
      downloading: false,
      percent: 100,
      error: null,
      indexing: false,
      indexed_meetings: 1,
      total_meetings: 1,
    },
    voicebook_view: [{ id: "v-marie", name: "Marie", samples: 12, meetings: 3 }],
    vocabulary_view: {
      dictionary: ["VoiceToText", "Aquadrox"],
      suggestions: [{ word: "Playwright", count: 3 }],
      snippets: [{ trigger: "mon mail", text: "contact@exemple.fr" }],
      styles: [{ name: "Code", apps: ["code"], titles: [], instructions: "Concis.", cleanup: true }],
    },
    prompts_view: {
      prompts: [],
      defaults: {
        cleanup: "Prompt nettoyage intégré.",
        command: "Prompt commande intégré.",
        summary: "Prompt compte rendu intégré.",
        conference: "Prompt conférence intégré.",
        report: "Ton clair, professionnel et factuel.",
      },
      cleanup_active: "",
      command_active: "",
      summary_active: "",
      conference_active: "",
      report_active: "",
    },
    app_settings_view: {
      hotkey: {
        key: "AltGr",
        command_key: "RightCtrl",
        paste_raw: "Alt+Shift+V",
        retry: "Alt+Shift+R",
        translate: "Alt+Shift+T",
        meeting: "Alt+Shift+M",
        meeting_copy: "Alt+Shift+N",
        annotate: "Alt+Shift+A",
        min_hold_ms: 150,
        hands_free: true,
        system_shortcut: false,
      },
      audio: { device: "", enhance: true },
      asr: {
        model: "parakeet-tdt-0.6b-v3-int8",
        use_gpu: true,
        unload_after_min: 15,
        unload_in_fullscreen: true,
      },
      dictation: {
        sounds: true,
        pause_media: true,
        voice_commands: true,
        screen_context: false,
        screen_context_excluded_apps: ["keepass"],
        live_insert: false,
      },
      translation: { enabled: false, language: "Anglais" },
      updates: { check: true },
      captures: {
        template: "explication",
        keep_days: 0,
        area: "ecran",
        clicks: true,
        vision: false,
        vision_profile: "",
        review_before_paste: false,
        redact_style: "flou",
      },
      autostart: false,
      microphones: ["Micro par défaut"],
      unload_choices: [0, 5, 15, 30, 60],
      capture_keep_days: [0, 1, 7, 30],
      llm_profiles: [],
      models: [
        { file: "parakeet-tdt-0.6b-v3-int8", label: "Parakeet TDT v3", engine: "parakeet", size_mb: 670, downloaded: true },
        { file: "large-v3-turbo-q8_0", label: "Whisper large-v3-turbo", engine: "whisper", size_mb: 874, downloaded: false },
      ],
      keys: ["AltGr", "RightCtrl", "F13", "Pause"],
      languages: ["Français", "Anglais", "Espagnol"],
      capture_templates: [["explication", "Explication"]],
      wayland: false,
      platform: "linux",
      version: "0.47.0",
    },
    current_status: { state: "idle" },
    meeting_state: { recording: false, elapsed_ms: 0 },
    annotate_state: { recording: false, elapsed_ms: 0 },
    annotate_review_view: {
      images: [
        {
          width: 320,
          height: 180,
          data_url:
            "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+ip1sAAAAASUVORK5CYII=",
        },
      ],
      style: "flou",
      vision_enabled: true,
      is_reedit: false,
    },
    translation_state: null,
    settings_view: {
      llm: {
        enabled: true,
        base_url: "https://api.deepseek.com/v1",
        model: "deepseek-chat",
        timeout_ms: 15000,
        cleanup_prompt: "",
        command_prompt: "",
        fallback_profile: "",
        pricing: { input_per_million: 0.27, cached_input_per_million: 0.07, output_per_million: 1.1, currency: "EUR" },
        extra_body: {},
      },
      profiles: [],
      presets: [
        { id: "deepseek", label: "DeepSeek", base_url: "https://api.deepseek.com/v1", model: "deepseek-chat" },
      ],
      has_key: true,
      key_host: "api.deepseek.com",
      api_keys: [{ host: "api.deepseek.com", provider: "DeepSeek", used_by: ["Réglage actif"] }],
      usage: {
        today: { words: 510, dictations: 12, requests: 12, cost: 0.02 },
        month: { words: 3200, dictations: 90, requests: 90, cost: 0.42 },
        total: { words: 12450, dictations: 420, requests: 420, cost: 1.8 },
        currency: "EUR",
      },
      prompts: [],
    },
    sync_view: {
      device_name: "PC-Test",
      device_id: "0123456789abcdef",
      listening: false,
      port: null,
      direct_pairing_enabled: false,
      peers: [],
      discovered: [],
      pending_direct: null,
    },
    update_status: { current: "0.43.0", available: null, checking: false, last_error: null },
    changelog_view: [{ version: "0.43.0", date: "25 novembre 2026", items: ["Onglet Rapport complet sur PC."] }],
  };
}

/** Ouvre `app.html` (ou `overlay.html`) avec un pont `window.__TAURI__` factice. */
export async function openUiPage(
  browser,
  baseUrl,
  {
    pagePath = "/app.html",
    hash = "",
    colorScheme = "light",
    viewport = { width: 980, height: 720 },
    mocks = defaultMockData(),
  } = {},
) {
  const context = await browser.newContext({ colorScheme, viewport });
  await context.addInitScript((initialMocks) => {
    const listeners = new Map();
    const calls = [];
    window.__VTT_CALLS__ = calls;
    window.__VTT_EMIT__ = (name, payload) => {
      for (const cb of listeners.get(name) || []) cb({ event: name, payload });
    };
    window.__TAURI__ = {
      core: {
        invoke: async (cmd, args) => {
          calls.push({ cmd, args: args ?? null });
          if (Object.prototype.hasOwnProperty.call(initialMocks, cmd)) {
            return structuredClone(initialMocks[cmd]);
          }
          return null;
        },
      },
      event: {
        listen: async (name, cb) => {
          if (!listeners.has(name)) listeners.set(name, []);
          listeners.get(name).push(cb);
          return () => {};
        },
      },
      window: {
        getCurrentWindow: () => ({
          hide: async () => {},
          show: async () => {},
          setFocus: async () => {},
        }),
      },
    };
  }, mocks);

  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (err) => errors.push(`pageerror: ${err.message}`));
  page.on("console", (msg) => {
    if (msg.type() === "error") errors.push(`console.error: ${msg.text()}`);
  });
  await page.goto(`${baseUrl}${pagePath}${hash}`, { waitUntil: "networkidle" });
  return { context, page, errors };
}
