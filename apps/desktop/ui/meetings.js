// Réunions : enregistrement, liste, transcription Moi / Eux, compte rendu,
// export, réglages. Le texte transcrit est toujours inséré avec textContent.
const meetingDateFormat = new Intl.DateTimeFormat("fr-FR", { dateStyle: "medium", timeStyle: "short" });
const MEETING_STATUS = {
  enregistrement: "enregistrement en cours",
  transcription: "transcription en cours…",
  terminee: "terminée",
  interrompue: "interrompue",
};
const DELETE_CONFIRM_MS = 4000;
const POLICIES = [
  ["demander", "Proposer d'enregistrer"],
  ["toujours", "Enregistrer sans demander"],
  ["jamais", "Ne rien proposer"],
];
let meetingsView = null;
let selectedMeeting = null;
let meetingSettingsFilled = false;
let recordingStartedAt = null;

/** « 12:34 » ou « 1:02:03 ». */
function clock(ms) {
  const seconds = Math.floor(ms / 1000);
  const h = Math.floor(seconds / 3600);
  const m = String(Math.floor(seconds / 60) % 60).padStart(2, "0");
  const s = String(seconds % 60).padStart(2, "0");
  return h > 0 ? `${h}:${m}:${s}` : `${m}:${s}`;
}

function showMeetingResult(text, isError = false) {
  const result = $("meeting-result");
  result.hidden = false;
  result.textContent = text;
  result.classList.toggle("error", isError);
}

async function showMeetings() {
  try {
    meetingsView = await invoke("meetings_view");
  } catch (e) {
    showMeetingResult(String(e), true);
    return;
  }
  const recording = meetingsView.recording;
  recordingStartedAt = recording ? Date.now() - meetingsView.elapsed_ms : null;
  $("meeting-toggle").textContent = recording ? "Arrêter la réunion" : "Démarrer une réunion";
  $("meeting-toggle").classList.toggle("danger", Boolean(recording));
  $("meeting-toggle").classList.toggle("primary", !recording);
  $("meeting-live").hidden = !recording;
  tickMeeting();
  renderMeetingList();
  if (!meetingSettingsFilled) fillMeetingSettings();
  if (!selectedMeeting && meetingsView.meetings.length) selectedMeeting = meetingsView.meetings[0].id;
  if (selectedMeeting && !meetingsView.meetings.some((m) => m.id === selectedMeeting)) selectedMeeting = null;
  await showMeetingDetail();
}

function renderMeetingList() {
  const items = meetingsView.meetings.map((meeting) => {
    const item = document.createElement("li");
    const button = document.createElement("button");
    button.type = "button";
    button.setAttribute("aria-current", String(meeting.id === selectedMeeting));
    const name = document.createElement("span");
    name.className = "name";
    name.textContent = meeting.title;
    const when = document.createElement("span");
    when.className = "when";
    const status = meeting.status === "terminee" ? clock(meeting.duration_ms) : MEETING_STATUS[meeting.status];
    when.textContent = `${meetingDateFormat.format(new Date(meeting.started_at_ms))} · ${status}`;
    button.append(name, when);
    button.addEventListener("click", () => {
      selectedMeeting = meeting.id;
      renderMeetingList();
      showMeetingDetail();
    });
    item.append(button);
    return item;
  });
  $("meeting-list").replaceChildren(...items);
  $("meeting-empty").hidden = items.length > 0;
}

async function showMeetingDetail() {
  const article = $("meeting-detail");
  if (!selectedMeeting) {
    article.hidden = true;
    return;
  }
  let detail;
  try {
    detail = await invoke("meeting_detail", { id: selectedMeeting });
  } catch (e) {
    article.hidden = true;
    showMeetingResult(String(e), true);
    return;
  }
  const { record } = detail;
  article.hidden = false;
  if (document.activeElement !== $("meeting-title")) $("meeting-title").value = record.title;
  const meta = [
    meetingDateFormat.format(new Date(record.started_at_ms)),
    `durée ${clock(record.duration_ms)}`,
    MEETING_STATUS[record.status],
    record.system ? "micro + son du PC" : "micro seulement",
  ];
  if (record.failed_chunks) meta.push(`${record.failed_chunks} passage(s) non transcrit(s)`);
  if (record.echo_chunks) meta.push(`${record.echo_chunks} passage(s) d'écho écarté(s)`);
  $("meeting-meta").textContent = meta.join(" · ");
  $("meeting-notes").hidden = record.notes.length === 0;
  $("meeting-notes").textContent = record.notes.join("\n");

  $("meeting-summarize").textContent = record.summary ? "Rédiger à nouveau" : "Rédiger le compte rendu";
  $("meeting-summarize").disabled = detail.busy || record.segments.length === 0 || !meetingsView.llm_enabled;
  $("meeting-summarize").title = meetingsView.llm_enabled ? "" : "Nettoyage LLM désactivé dans les réglages";
  $("meeting-audio-me").hidden = !detail.audio_kept;
  $("meeting-audio-them").hidden = !(detail.audio_kept && record.system);
  $("meeting-retranscribe").hidden = !detail.audio_kept;
  $("meeting-retranscribe").disabled = detail.busy;
  $("meeting-delete").disabled = detail.busy;

  $("meeting-summary-block").hidden = !record.summary;
  if (record.summary) {
    const cost = record.summary.cost != null ? ` · ${formatCost(record.summary.cost, record.summary.currency)}` : "";
    $("meeting-summary-meta").textContent =
      `${meetingDateFormat.format(new Date(record.summary.at_ms))} · ${record.summary.model}${cost}`;
    $("meeting-summary").replaceChildren(...renderSummary(record.summary.text));
  }
  $("meeting-summary-error").hidden = !record.summary_error;
  $("meeting-summary-error").textContent = record.summary_error ? `Compte rendu impossible : ${record.summary_error}` : "";

  const turns = detail.turns.map((turn) => {
    const item = document.createElement("li");
    item.dataset.track = turn.track;
    const time = document.createElement("span");
    time.className = "time";
    time.textContent = clock(turn.start_ms);
    const who = document.createElement("span");
    who.className = "who";
    who.textContent = turn.track === "moi" ? "Moi" : "Eux";
    const text = document.createElement("span");
    text.className = "text";
    text.textContent = turn.text;
    item.append(time, who, text);
    return item;
  });
  $("meeting-turns").replaceChildren(...turns);
  $("meeting-turns-empty").hidden = turns.length > 0;
  $("meeting-turns-empty").textContent =
    record.status === "terminee" ? "Aucune parole détectée." : "La transcription s'affiche ici au fil de la réunion.";
}

/**
 * Markdown du compte rendu (titres, listes, paragraphes, gras), construit
 * élément par élément avec textContent : rien n'est interprété comme du HTML.
 */
function renderSummary(markdown) {
  const nodes = [];
  let list = null;
  const inline = (element, text) => {
    // **gras** seulement.
    text.split(/\*\*(.+?)\*\*/).forEach((part, i) => {
      if (!part) return;
      if (i % 2) {
        const strong = document.createElement("strong");
        strong.textContent = part;
        element.append(strong);
      } else {
        element.append(part);
      }
    });
    return element;
  };
  for (const raw of markdown.split("\n")) {
    const line = raw.trim();
    const heading = line.match(/^#{1,6}\s+(.*)$/);
    const item = line.match(/^[-*•]\s+(.*)$/) ?? line.match(/^\d+[.)]\s+(.*)$/);
    if (item) {
      if (!list) {
        list = document.createElement("ul");
        nodes.push(list);
      }
      list.append(inline(document.createElement("li"), item[1]));
      continue;
    }
    list = null;
    if (heading) nodes.push(inline(document.createElement("h3"), heading[1]));
    else if (line) nodes.push(inline(document.createElement("p"), line));
  }
  return nodes;
}

/** Langues proposées pour le compte rendu ; `selected` reste proposée. */
function languageOptions(selected) {
  const languages = [...meetingsView.languages];
  if (selected && !languages.includes(selected)) languages.push(selected);
  return [option("", "Langue de la réunion"), ...languages.map((l) => option(l, l))];
}

function fillMeetingSettings() {
  const settings = meetingsView.settings;
  $("meeting-echo").checked = settings.echo_filter;
  $("meeting-detect").checked = settings.detect;
  const rows = meetingsView.providers.map((provider) => {
    const row = document.createElement("tr");
    const name = document.createElement("th");
    name.textContent = provider.browser ? `${provider.label} (navigateur)` : provider.label;
    const cell = document.createElement("td");
    const select = document.createElement("select");
    select.dataset.provider = provider.key;
    select.setAttribute("aria-label", `Appels ${provider.label}`);
    select.replaceChildren(...POLICIES.map(([value, label]) => option(value, label)));
    select.value = provider.policy;
    cell.append(select);
    row.append(name, cell);
    return row;
  });
  $("meeting-providers").tBodies[0].replaceChildren(...rows);
  $("meeting-language").replaceChildren(...languageOptions(settings.summary_language));
  $("meeting-language").value = settings.summary_language;
  $("meeting-summary-language").replaceChildren(...languageOptions(settings.summary_language));
  $("meeting-summary-language").value = settings.summary_language;
  for (const radio of document.querySelectorAll('input[name="meeting-audio"]')) {
    radio.checked = radio.value === settings.audio;
  }
  $("meeting-system").checked = settings.system_audio;
  $("meeting-auto-summary").checked = settings.auto_summary;
  const days = meetingsView.keep_days.includes(settings.keep_days)
    ? meetingsView.keep_days
    : [...meetingsView.keep_days, settings.keep_days].sort((a, b) => a - b);
  $("meeting-keep").replaceChildren(...days.map((d) => option(String(d), d === 0 ? "Toujours" : `${d} jours`)));
  $("meeting-keep").value = String(settings.keep_days);
  meetingSettingsFilled = true;
}

function tickMeeting() {
  if (recordingStartedAt != null) $("meeting-elapsed").textContent = clock(Date.now() - recordingStartedAt);
}
setInterval(tickMeeting, 1000);

/** Action sur la réunion affichée, bouton désactivé pendant l'appel. */
async function meetingAction(button, action) {
  button.disabled = true;
  try {
    await action();
  } catch (e) {
    showMeetingResult(String(e), true);
  } finally {
    button.disabled = false;
    await showMeetings();
  }
}

$("meeting-toggle").addEventListener("click", () =>
  meetingAction($("meeting-toggle"), async () => {
    if (meetingsView?.recording) {
      await invoke("stop_meeting");
      showMeetingResult("Enregistrement arrêté : fin de la transcription en cours.");
    } else {
      selectedMeeting = await invoke("start_meeting");
      showMeetingResult("Réunion enregistrée : pensez à prévenir les participants.");
    }
  }),
);

async function renameMeeting() {
  const title = $("meeting-title").value.trim();
  const current = meetingsView?.meetings.find((m) => m.id === selectedMeeting);
  if (!current || !title || title === current.title) return;
  try {
    await invoke("rename_meeting", { id: selectedMeeting, title });
    await showMeetings();
  } catch (e) {
    showMeetingResult(String(e), true);
  }
}
$("meeting-title").addEventListener("change", renameMeeting);
$("meeting-title").addEventListener("keydown", (e) => {
  if (e.key === "Enter") $("meeting-title").blur();
});

$("meeting-summarize").addEventListener("click", () =>
  meetingAction($("meeting-summarize"), async () => {
    showMeetingResult("Rédaction du compte rendu par le LLM…");
    await invoke("summarize_meeting", {
      id: selectedMeeting,
      language: $("meeting-summary-language").value,
    });
    showMeetingResult("✓ Compte rendu rédigé.");
  }),
);

$("meeting-copy").addEventListener("click", () =>
  meetingAction($("meeting-copy"), async () => {
    await invoke("copy_meeting", { id: selectedMeeting, part: "all" });
    showMeetingResult("✓ Compte rendu et transcription copiés (Markdown).");
  }),
);

$("meeting-export").addEventListener("click", () =>
  meetingAction($("meeting-export"), async () => {
    const path = await invoke("export_meeting", { id: selectedMeeting });
    if (path) showMeetingResult(`✓ Exporté : ${path}`);
  }),
);

for (const [id, track] of [
  ["meeting-audio-me", "moi"],
  ["meeting-audio-them", "eux"],
]) {
  $(id).addEventListener("click", () =>
    meetingAction($(id), async () => {
      const path = await invoke("export_meeting_audio", { id: selectedMeeting, track });
      if (path) showMeetingResult(`✓ Audio exporté (non chiffré) : ${path}`);
    }),
  );
}

$("meeting-retranscribe").addEventListener("click", () =>
  meetingAction($("meeting-retranscribe"), async () => {
    await invoke("retranscribe_meeting", { id: selectedMeeting });
    showMeetingResult("Nouvelle transcription en cours, avec le modèle actuel.");
  }),
);

// Suppression en deux clics.
let meetingDeleteTimer = null;
$("meeting-delete").addEventListener("click", () => {
  const button = $("meeting-delete");
  if (!meetingDeleteTimer) {
    button.textContent = "Confirmer la suppression";
    meetingDeleteTimer = setTimeout(() => {
      button.textContent = "Supprimer";
      meetingDeleteTimer = null;
    }, DELETE_CONFIRM_MS);
    return;
  }
  clearTimeout(meetingDeleteTimer);
  meetingDeleteTimer = null;
  button.textContent = "Supprimer";
  meetingAction(button, async () => {
    await invoke("delete_meeting", { id: selectedMeeting });
    selectedMeeting = null;
    showMeetingResult("Réunion supprimée (transcription, compte rendu et audio).");
  });
});

$("meeting-settings-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const result = $("meeting-settings-result");
  result.hidden = false;
  try {
    await invoke("save_meeting_settings", {
      settings: {
        audio: document.querySelector('input[name="meeting-audio"]:checked')?.value ?? "temporaire",
        system_audio: $("meeting-system").checked,
        auto_summary: $("meeting-auto-summary").checked,
        summary_prompt: "",
        keep_days: Number($("meeting-keep").value),
        detect: $("meeting-detect").checked,
        echo_filter: $("meeting-echo").checked,
        summary_language: $("meeting-language").value,
        providers: [...document.querySelectorAll("[data-provider]")]
          .filter((select) => select.value !== "demander")
          .map((select) => ({ key: select.dataset.provider, policy: select.value })),
      },
    });
    result.classList.remove("error");
    result.textContent = "✓ Appliqué (prochaine réunion pour l'audio).";
    meetingSettingsFilled = false;
    await showMeetings();
  } catch (e) {
    result.classList.add("error");
    result.textContent = String(e);
  }
});

// Transcription au fil de l'eau : la page suit les blocs transcrits.
let meetingsRefresh = null;
window.__TAURI__.event.listen("meetings", () => {
  if (!isShown("reunions")) return;
  clearTimeout(meetingsRefresh);
  meetingsRefresh = setTimeout(showMeetings, 300);
});
window.__TAURI__.event.listen("meeting", () => isShown("reunions") && showMeetings());

registerPage("reunions", {
  show: async () => {
    try {
      const settings = await invoke("app_settings_view");
      const keys = [settings.hotkey.meeting, settings.hotkey.meeting_copy].map((k) => k || "désactivé");
      $("meeting-shortcuts").textContent = `démarrer / arrêter ${keys[0]}, coller les nouvelles répliques ${keys[1]}`;
    } catch {
      $("meeting-shortcuts").textContent = "voir Réglages → Raccourcis";
    }
    await showMeetings();
  },
});
