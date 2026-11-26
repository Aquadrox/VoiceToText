// Réunions : enregistrement, liste, transcription Moi / Eux, compte rendu,
// export, réglages (section Réunions des Réglages). Le texte transcrit est toujours inséré avec textContent.
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
/** Passage à mettre en évidence après un clic sur un résultat (ms). */
let highlightAt = null;
let activeMeetingTab = "transcript";
let currentDetail = null;
let pendingReportSearch = null;
let reportFrameKey = "";
let reportPlayingMs = -1;
const KIND_LABELS = { reunion: "Réunion", conference: "Conférence", fichier: "Fichier" };

/** Nom de la piste selon le type d'enregistrement (vide : une seule source). */
function trackLabel(record, track) {
  if (record.kind === "reunion") return track === "moi" ? "Moi" : "Eux";
  if (record.kind === "conference" && record.mic && record.system) return track === "moi" ? "Micro" : "PC";
  return "";
}

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
  $("meeting-toggle").textContent = recording ? "Arrêter l'enregistrement" : "Démarrer une réunion";
  $("conf-start").disabled = Boolean(recording);
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
    const status =
      meeting.progress != null
        ? `lecture ${meeting.progress} %`
        : meeting.identifying
          ? "intervenants…"
          : meeting.status === "terminee"
          ? clock(meeting.duration_ms)
          : MEETING_STATUS[meeting.status];
    const kind = meeting.kind === "reunion" ? "" : `${KIND_LABELS[meeting.kind]} · `;
    const hasReport = meeting.has_report ? " · rapport" : "";
    when.textContent = `${kind}${meetingDateFormat.format(new Date(meeting.started_at_ms))} · ${status}${hasReport}`;
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
    currentDetail = null;
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
  currentDetail = detail;
  const { record } = detail;
  article.hidden = false;
  if (article.dataset.id !== record.id) {
    // Autre enregistrement : le lecteur repart de zéro.
    article.dataset.id = record.id;
    reportFrameKey = "";
    reportPlayingMs = -1;
    $("meeting-player").pause();
    $("meeting-player").hidden = true;
  }
  if (document.activeElement !== $("meeting-title")) $("meeting-title").value = record.title;
  const sources =
    record.kind === "fichier"
      ? `fichier ${record.source ?? ""}`
      : record.mic && record.system
        ? "micro + son du PC"
        : record.system
          ? "son du PC seulement"
          : "micro seulement";
  const meta = [
    KIND_LABELS[record.kind],
    meetingDateFormat.format(new Date(record.started_at_ms)),
    `durée ${clock(record.duration_ms)}`,
    record.progress != null
      ? `lecture du fichier ${record.progress} %`
      : record.identifying
        ? "identification des intervenants…"
        : MEETING_STATUS[record.status],
    sources,
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
  $("meeting-retranscribe").disabled = detail.busy || record.identifying;
  $("meeting-identify").hidden = !detail.can_identify;
  $("meeting-delete").disabled = detail.busy || record.identifying;

  $("meeting-summary-block").hidden = !record.summary;
  if (record.summary) {
    const cost = record.summary.cost != null ? ` · ${formatCost(record.summary.cost, record.summary.currency)}` : "";
    $("meeting-summary-meta").textContent =
      `${meetingDateFormat.format(new Date(record.summary.at_ms))} · ${record.summary.model}${cost}`;
    $("meeting-summary").replaceChildren(...renderSummary(record.summary.text));
  }
  $("meeting-summary-error").hidden = !record.summary_error;
  $("meeting-summary-error").textContent = record.summary_error ? `Compte rendu impossible : ${record.summary_error}` : "";

  renderReportSection(detail);

  renderSpeakers(detail);
  $("meeting-turns").classList.toggle("named", detail.speakers.length > 0);
  const turns = detail.turns.map((turn) => {
    const item = document.createElement("li");
    item.dataset.track = turn.track;
    if (turn.speaker != null) item.dataset.speaker = String(turn.speaker % SPEAKER_COLORS);
    item.dataset.start = turn.start_ms;
    // Audio disponible : l'heure devient un bouton qui fait réécouter.
    const time = document.createElement(detail.replay ? "button" : "span");
    time.className = "time";
    time.textContent = detail.replay ? `▶ ${clock(turn.start_ms)}` : clock(turn.start_ms);
    if (detail.replay) {
      time.type = "button";
      time.title = "Réécouter ce passage";
      time.addEventListener("click", () => playClip(record.id, turn.start_ms, time));
    }
    const who = document.createElement("span");
    who.className = "who";
    who.textContent = turn.who || trackLabel(record, turn.track);
    who.title = who.textContent;
    const text = document.createElement("span");
    text.className = "text";
    text.textContent = turn.text;
    item.append(time, who, text);
    return item;
  });
  $("meeting-turns").replaceChildren(...turns);
  if (highlightAt != null) {
    // Réplique qui contient le passage trouvé : la dernière qui commence avant.
    const target = turns.filter((t) => Number(t.dataset.start) <= highlightAt).pop() ?? turns[0];
    highlightAt = null;
    if (target) {
      target.classList.add("highlight");
      target.scrollIntoView({ block: "center" });
    }
  }
  $("meeting-turns-empty").hidden = turns.length > 0;
  $("meeting-turns-empty").textContent =
    record.status === "terminee" ? "Aucune parole détectée." : "La transcription s'affiche ici au fil de la réunion.";
}

/** Couleurs des intervenants (app.css), réutilisées au-delà. */
const SPEAKER_COLORS = 6;
/** Nombre d'intervenants proposé au plus (et au moins jusqu'à ceux trouvés). */
const MAX_SPEAKER_CHOICE = 8;

/** Libellé du bouton de mémorisation d'une voix d'intervenant. */
function enrollButtonLabel(name, recognized) {
  const trimmed = (name || "").trim();
  if (!trimmed) return "Mémoriser la voix";
  return recognized ? `Enrichir la voix de ${trimmed}` : `Mémoriser la voix de ${trimmed}`;
}

/** Intervenants distingués : nom à donner, temps de parole, première phrase. */
function renderSpeakers(detail) {
  const { record, speakers } = detail;
  $("meeting-speakers-block").hidden = speakers.length === 0;
  if (speakers.length === 0) return;
  const list = $("meeting-speakers");
  const canEnroll = Boolean(detail.voices_kept && !detail.busy && !record.identifying);
  // Nom en cours de saisie : la liste n'est pas reconstruite sous les doigts.
  if (!list.contains(document.activeElement) || list.dataset.id !== record.id) {
    list.dataset.id = record.id;
    list.replaceChildren(
      ...speakers.map((speaker) => {
        const item = document.createElement("li");
        item.dataset.speaker = String(speaker.index % SPEAKER_COLORS);
        const label = document.createElement("span");
        label.className = "label";
        label.textContent = speaker.label;
        const input = document.createElement("input");
        input.type = "text";
        input.value = speaker.name;
        input.placeholder = "Nom";
        input.maxLength = 60;
        input.spellcheck = false;
        input.setAttribute("aria-label", `Nom de ${speaker.label}`);
        input.addEventListener("change", () => renameSpeaker(record.id, speaker.index, input.value));
        input.addEventListener("keydown", (e) => {
          if (e.key === "Enter") input.blur();
        });
        const about = document.createElement("span");
        about.className = "about";
        about.textContent = `${clock(speaker.talk_ms)} de parole · « ${speaker.first} »`;
        item.append(label, input, about);
        if (speaker.recognized) {
          input.classList.add("recognized");
          input.title = `Voix reconnue dans votre carnet local (${speaker.recognized.confidence} %) : modifiez ou effacez si besoin`;
          const rec = document.createElement("span");
          rec.className = "recognized";
          rec.textContent = `Voix reconnue dans le carnet (${speaker.recognized.confidence} %)`;
          item.append(rec);
        } else if (speaker.found) {
          // Nom trouvé quand l'intervenant s'est présenté : d'où il vient.
          input.classList.add("found");
          input.title = "Nom trouvé dans sa présentation : modifiez-le ou effacez-le si besoin";
          const found = document.createElement("span");
          found.className = "found";
          found.textContent = `Présentation à ${clock(speaker.found_ms)} : « ${speaker.found} »`;
          item.append(found);
        }
        if (canEnroll) {
          const actions = document.createElement("div");
          actions.className = "speaker-actions";
          const enroll = document.createElement("button");
          enroll.type = "button";
          enroll.className = "speaker-enroll";
          const syncEnroll = () => {
            const hasName = Boolean(input.value.trim());
            enroll.textContent = enrollButtonLabel(input.value, Boolean(speaker.recognized));
            enroll.disabled = !hasName;
            enroll.title = hasName
              ? "Mémorise cette empreinte vocale dans votre carnet local chiffré"
              : "Donnez d'abord un nom à cet intervenant pour mémoriser sa voix";
          };
          syncEnroll();
          input.addEventListener("input", syncEnroll);
          enroll.addEventListener("click", () =>
            enrollSpeakerVoice(record.id, speaker.index, input.value, enroll),
          );
          actions.append(enroll);
          item.append(actions);
        }
        return item;
      }),
    );
  }
  const select = $("meeting-speaker-count");
  const most = Math.max(MAX_SPEAKER_CHOICE, speakers.length);
  select.replaceChildren(
    option("", "Selon les voix"),
    ...Array.from({ length: most }, (_, i) => option(String(i + 1), String(i + 1))),
  );
  select.value = record.speaker_count == null ? "" : String(record.speaker_count);
  select.disabled = !detail.voices_kept || detail.busy || record.identifying;
  select.title = detail.voices_kept
    ? ""
    : "Empreintes vocales non conservées (audio en mémoire seulement, ou analyse effacée)";
  $("meeting-forget-speakers").disabled = detail.busy || record.identifying;
}

async function renameSpeaker(id, index, name) {
  try {
    await invoke("rename_speaker", { id, index, name });
    await showMeetingDetail();
  } catch (e) {
    showMeetingResult(String(e), true);
  }
}

async function enrollSpeakerVoice(id, index, name, button) {
  button.disabled = true;
  try {
    const entry = await invoke("enroll_speaker_voice", { id, index, name });
    showMeetingResult(
      entry.meetings > 1
        ? `Voix de ${entry.name} enrichie dans le carnet local (${entry.meetings} réunions).`
        : `Voix de ${entry.name} mémorisée dans le carnet local (chiffré).`,
    );
    await showMeetingDetail();
    await showVoicebook();
  } catch (e) {
    showMeetingResult(String(e), true);
  } finally {
    button.disabled = false;
  }
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

/** Réécoute : extrait audio à partir d'un peu avant le passage. */
async function playClip(id, startMs, button) {
  const player = $("meeting-player");
  button.disabled = true;
  try {
    player.src = await invoke("meeting_clip", { id, startMs });
    player.hidden = false;
    await player.play();
  } catch (e) {
    showMeetingResult(String(e), true);
  } finally {
    button.disabled = false;
  }
}

function fillMeetingSettings() {
  const settings = meetingsView.settings;
  $("meeting-echo").checked = settings.echo_filter;
  $("meeting-diarize").checked = settings.diarize;
  showVoices();
  if ($("meeting-semantic")) {
    $("meeting-semantic").checked = Boolean(settings.semantic_search);
    showSemantic();
  }
  $("meeting-detect").checked = settings.detect;
  if ($("meeting-auto-stop")) {
    const autoStop = settings.auto_stop ?? true;
    $("meeting-auto-stop").checked = autoStop;
    const delaySelect = $("meeting-auto-stop-delay");
    if (delaySelect) {
      const currentDelay = settings.auto_stop_delay_sec ?? 15;
      const baseDelays = meetingsView.auto_stop_delays ?? [10, 15, 30, 60];
      const delays = baseDelays.includes(currentDelay)
        ? baseDelays
        : [...baseDelays, currentDelay].sort((a, b) => a - b);
      delaySelect.replaceChildren(...delays.map((s) => option(String(s), `${s} s`)));
      delaySelect.value = String(currentDelay);
      delaySelect.disabled = !autoStop;
    }
  }
  if ($("meeting-auto-stop-silence")) {
    const currentSilence = settings.auto_stop_silence_min ?? 0;
    const baseSilence = meetingsView.auto_stop_silence_mins ?? [0, 2, 5, 10, 15];
    const mins = baseSilence.includes(currentSilence)
      ? baseSilence
      : [...baseSilence, currentSilence].sort((a, b) => a - b);
    $("meeting-auto-stop-silence").replaceChildren(
      ...mins.map((m) => option(String(m), m === 0 ? "Désactivé" : `${m} min`)),
    );
    $("meeting-auto-stop-silence").value = String(currentSilence);
  }
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
  SettingsUI.markClean("reunions");
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

$("conf-start").addEventListener("click", () =>
  meetingAction($("conf-start"), async () => {
    selectedMeeting = await invoke("start_conference", {
      mic: $("conf-mic").checked,
      system: $("conf-system").checked,
      output: $("conf-output").value,
    });
    showMeetingResult("Conférence enregistrée : « Arrêter l'enregistrement » à la fin.");
  }),
);

$("import-file").addEventListener("click", () =>
  meetingAction($("import-file"), async () => {
    const id = await invoke("import_recording", { output: $("conf-output").value });
    if (!id) return;
    selectedMeeting = id;
    showMeetingResult("Fichier en cours de transcription (progression dans la liste).");
  }),
);

/* ---------- Recherche ---------- */

let searchTimer = null;
$("meeting-search").addEventListener("input", () => {
  clearTimeout(searchTimer);
  searchTimer = setTimeout(runSearch, 300);
});

async function runSearch() {
  const query = $("meeting-search").value.trim();
  if (query.length < 2) {
    $("search-results").hidden = true;
    $("search-empty").hidden = true;
    return;
  }
  let results;
  try {
    results = await invoke("search_meetings", { query });
  } catch (e) {
    showMeetingResult(String(e), true);
    return;
  }
  const items = results.map((result) => {
    const item = document.createElement("li");
    const button = document.createElement("button");
    button.type = "button";
    const where = document.createElement("span");
    where.className = "where";
    if (result.source === "report") {
      const time = result.start_ms != null ? ` · ${clock(result.start_ms)}` : "";
      where.textContent = `${result.title} · Rapport (${result.section || "Rapport"})${time}`;
    } else {
      where.textContent =
        `${result.title} · ${meetingDateFormat.format(new Date(result.started_at_ms))} · ${clock(result.start_ms ?? 0)}`;
    }
    if (result.match_kind === "semantic" || result.match_kind === "hybrid") {
      const badge = document.createElement("span");
      badge.className = `search-badge ${result.match_kind}`;
      badge.textContent = result.match_kind === "hybrid" ? "Exact + sens" : "Sens proche";
      if (typeof result.semantic_score === "number") {
        badge.title = `Similarité sémantique : ${Math.round(result.semantic_score * 100)} %`;
      }
      where.append(badge);
    }
    const text = document.createElement("span");
    for (const [part, marked] of result.parts) {
      if (marked) {
        const mark = document.createElement("mark");
        mark.textContent = part;
        text.append(mark);
      } else {
        text.append(part);
      }
    }
    button.append(where, text);
    button.addEventListener("click", async () => {
      selectedMeeting = result.id;
      if (result.source === "report") {
        highlightAt = null;
        pendingReportSearch = query;
        setMeetingTab("report");
      } else {
        highlightAt = result.start_ms;
        setMeetingTab("transcript");
      }
      renderMeetingList();
      await showMeetingDetail();
    });
    item.append(button);
    return item;
  });
  $("search-results").replaceChildren(...items);
  $("search-results").hidden = items.length === 0;
  $("search-empty").hidden = items.length > 0;
}

/* ---------- Questions au LLM ---------- */

$("ask-send").addEventListener("click", async () => {
  const button = $("ask-send");
  const answer = $("ask-answer");
  const question = $("ask-question").value.trim();
  if (!question) return;
  button.disabled = true;
  answer.hidden = false;
  answer.classList.remove("error");
  answer.textContent = "Recherche des passages et réponse du LLM…";
  try {
    const result = await invoke("ask_meetings", {
      question,
      id: $("ask-scope").value === "one" ? selectedMeeting : null,
    });
    const sem = result.semantic ? " · recherche sémantique" : "";
    const cost = result.cost != null ? ` · ${formatCost(result.cost, result.currency)}` : "";
    answer.textContent = `${result.text}\n\n(${result.sources} enregistrement(s) consulté(s)${sem}${cost})`;
  } catch (e) {
    answer.classList.add("error");
    answer.textContent = String(e);
  } finally {
    button.disabled = false;
  }
});
$("ask-question").addEventListener("keydown", (e) => {
  if (e.key === "Enter") $("ask-send").click();
});

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

$("meeting-identify").addEventListener("click", () =>
  meetingAction($("meeting-identify"), async () => {
    await invoke("identify_speakers", { id: selectedMeeting });
    showMeetingResult("Identification des intervenants en cours (quelques minutes pour une heure d'audio).");
  }),
);

$("meeting-speaker-count").addEventListener("change", () =>
  meetingAction($("meeting-speaker-count"), async () => {
    const value = $("meeting-speaker-count").value;
    await invoke("set_speaker_count", { id: selectedMeeting, count: value ? Number(value) : null });
    showMeetingResult(value ? `✓ ${value} intervenant(s).` : "✓ Nombre d'intervenants déduit des voix.");
  }),
);

// « Ne plus distinguer » en deux clics : les empreintes vocales sont effacées.
let forgetSpeakersTimer = null;
$("meeting-forget-speakers").addEventListener("click", () => {
  const button = $("meeting-forget-speakers");
  if (!forgetSpeakersTimer) {
    button.textContent = "Confirmer";
    forgetSpeakersTimer = setTimeout(() => {
      button.textContent = "Ne plus distinguer";
      forgetSpeakersTimer = null;
    }, DELETE_CONFIRM_MS);
    return;
  }
  clearTimeout(forgetSpeakersTimer);
  forgetSpeakersTimer = null;
  button.textContent = "Ne plus distinguer";
  meetingAction(button, async () => {
    await invoke("forget_speakers", { id: selectedMeeting });
    showMeetingResult("Intervenants et empreintes vocales effacés : « Eux » à nouveau.");
  });
});

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

/** Section Réunions des réglages : remplie depuis la liste des réunions. */
async function loadMeetingSettings() {
  showVoicebook();
  if (meetingSettingsFilled) return;
  try {
    meetingsView = await invoke("meetings_view");
    if (!meetingSettingsFilled) fillMeetingSettings();
  } catch (e) {
    SettingsUI.showResult("reunions", String(e), true);
  }
}

SettingsUI.define("reunions", {
  apply: async () => {
    await invoke("save_meeting_settings", {
      settings: {
        audio: document.querySelector('input[name="meeting-audio"]:checked')?.value ?? "temporaire",
        system_audio: $("meeting-system").checked,
        auto_summary: $("meeting-auto-summary").checked,
        summary_prompt: "",
        keep_days: Number($("meeting-keep").value),
        detect: $("meeting-detect").checked,
        auto_stop: $("meeting-auto-stop") ? $("meeting-auto-stop").checked : true,
        auto_stop_delay_sec: $("meeting-auto-stop-delay") ? Number($("meeting-auto-stop-delay").value) : 15,
        auto_stop_silence_min: $("meeting-auto-stop-silence") ? Number($("meeting-auto-stop-silence").value) : 0,
        echo_filter: $("meeting-echo").checked,
        summary_language: $("meeting-language").value,
        diarize: $("meeting-diarize").checked,
        semantic_search: $("meeting-semantic") ? $("meeting-semantic").checked : false,
        providers: [...document.querySelectorAll("[data-provider]")]
          .filter((select) => select.value !== "demander")
          .map((select) => ({ key: select.dataset.provider, policy: select.value })),
      },
    });
    meetingSettingsFilled = false;
    await loadMeetingSettings();
    const voices = await invoke("voices_view").catch(() => null);
    if ($("meeting-diarize").checked && voices && !voices.installed) {
      return "✓ Appliqué ; téléchargement des modèles des intervenants en cours.";
    }
    const sem = await invoke("semantic_view").catch(() => null);
    if ($("meeting-semantic")?.checked && sem && !sem.installed) {
      return "✓ Appliqué ; téléchargement du modèle de recherche sémantique en cours.";
    }
    return "✓ Appliqué (l'audio et le son du PC : dès la prochaine réunion).";
  },
});

if ($("meeting-auto-stop")) {
  $("meeting-auto-stop").addEventListener("change", () => {
    if ($("meeting-auto-stop-delay")) {
      $("meeting-auto-stop-delay").disabled = !$("meeting-auto-stop").checked;
    }
  });
}

/** État des modèles des intervenants (section Réunions des réglages). */
async function showVoices(view) {
  try {
    view ??= await invoke("voices_view");
  } catch {
    return;
  }
  const status = $("meeting-diarize-status");
  const button = $("meeting-diarize-download");
  button.hidden = true;
  if (view.installed) {
    status.textContent = "Modèles installés.";
  } else if (view.downloading) {
    status.textContent = `Téléchargement des modèles : ${view.percent} %`;
  } else if (view.error) {
    status.textContent = `Téléchargement impossible : ${view.error}`;
    button.textContent = "Réessayer";
    button.hidden = false;
  } else {
    status.textContent = `${view.missing_mb} Mo à télécharger à l'activation (vérifiés avant usage).`;
    button.textContent = "Télécharger les modèles";
    button.hidden = !$("meeting-diarize").checked;
  }
}
$("meeting-diarize-download").addEventListener("click", async () => {
  try {
    await invoke("download_voices");
  } finally {
    await showVoices();
  }
});
$("meeting-diarize").addEventListener("change", () => showVoices());
window.__TAURI__.event.listen("voices", (event) => showVoices(event.payload));

/** État du modèle et de l'index de recherche sémantique locale (Réglages → Réunions). */
async function showSemantic(view) {
  const checkbox = $("meeting-semantic");
  const status = $("meeting-semantic-status");
  const dlButton = $("meeting-semantic-download");
  const reindexButton = $("meeting-semantic-reindex");
  if (!checkbox || !status || !dlButton || !reindexButton) return;
  try {
    view ??= await invoke("semantic_view");
  } catch {
    return;
  }
  dlButton.hidden = true;
  reindexButton.hidden = true;
  if (view.downloading) {
    status.textContent = `Téléchargement du modèle : ${view.percent} %`;
  } else if (view.installed) {
    if (view.indexing) {
      status.textContent = `Modèle installé · indexation en cours (${view.indexed_meetings} / ${view.total_meetings} réunions)…`;
    } else if (view.total_meetings > 0) {
      status.textContent = `Modèle installé · ${view.indexed_meetings} / ${view.total_meetings} réunions indexées.`;
      reindexButton.hidden = false;
    } else {
      status.textContent = "Modèle installé.";
    }
  } else if (view.error) {
    status.textContent = `Téléchargement impossible : ${view.error}`;
    dlButton.textContent = "Réessayer";
    dlButton.hidden = false;
  } else {
    status.textContent = `${view.missing_mb} Mo à télécharger à l'activation (vérifiés avant usage).`;
    dlButton.textContent = "Télécharger le modèle";
    dlButton.hidden = !checkbox.checked;
  }
}
if ($("meeting-semantic-download")) {
  $("meeting-semantic-download").addEventListener("click", async () => {
    try {
      await invoke("download_semantic");
    } finally {
      await showSemantic();
    }
  });
}
if ($("meeting-semantic-reindex")) {
  $("meeting-semantic-reindex").addEventListener("click", async () => {
    try {
      await invoke("reindex_semantic");
    } finally {
      await showSemantic();
    }
  });
}
if ($("meeting-semantic")) {
  $("meeting-semantic").addEventListener("change", () => showSemantic());
}
window.__TAURI__.event.listen("semantic", (event) => showSemantic(event.payload));

/** Résultat d'une action sur le carnet de voix local (Réglages → Réunions). */
function showVoicebookResult(text, isError = false) {
  const box = $("voicebook-result");
  if (!box) return;
  box.textContent = text;
  box.classList.toggle("err", isError);
  box.hidden = !text;
}

function formatVoiceMeta(entry) {
  const meetings = `${entry.meetings} réunion${entry.meetings > 1 ? "s" : ""}`;
  const samples = `${entry.samples} extrait${entry.samples > 1 ? "s" : ""}`;
  return `${meetings} · ${samples}`;
}

/** Liste des voix mémorisées dans Réglages → Réunions. */
async function showVoicebook(entries) {
  const list = $("voicebook-list");
  if (!list) return;
  try {
    entries ??= await invoke("voicebook_view");
  } catch (e) {
    showVoicebookResult(String(e), true);
    return;
  }
  $("voicebook-empty").hidden = entries.length > 0;
  $("voicebook-clear").hidden = entries.length === 0;
  if (list.contains(document.activeElement)) return;
  list.replaceChildren(
    ...entries.map((entry) => {
      const item = document.createElement("li");
      const input = document.createElement("input");
      input.type = "text";
      input.className = "voicebook-name";
      input.value = entry.name;
      input.maxLength = 60;
      input.spellcheck = false;
      input.setAttribute("aria-label", `Nom de la voix ${entry.name}`);
      input.addEventListener("change", () => renameVoicebookEntry(entry.id, input.value));
      input.addEventListener("keydown", (e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          input.blur();
        }
      });
      const meta = document.createElement("span");
      meta.className = "voicebook-meta";
      meta.textContent = formatVoiceMeta(entry);
      const remove = document.createElement("button");
      remove.type = "button";
      remove.className = "danger voicebook-remove";
      remove.textContent = "Oublier";
      remove.title = `Supprimer immédiatement la voix de ${entry.name}`;
      remove.addEventListener("click", () => removeVoicebookEntry(entry.id, entry.name, remove));
      item.append(input, meta, remove);
      return item;
    }),
  );
}

async function renameVoicebookEntry(id, name) {
  try {
    await invoke("rename_voicebook_entry", { id, name });
    showVoicebookResult("Nom mis à jour dans le carnet.");
  } catch (e) {
    showVoicebookResult(String(e), true);
  } finally {
    await showVoicebook();
  }
}

async function removeVoicebookEntry(id, name, button) {
  button.disabled = true;
  try {
    await invoke("remove_voicebook_entry", { id });
    showVoicebookResult(`Voix de ${name} oubliée.`);
  } catch (e) {
    showVoicebookResult(String(e), true);
  } finally {
    await showVoicebook();
  }
}

let voicebookClearArmed = null;
$("voicebook-clear")?.addEventListener("click", async () => {
  const button = $("voicebook-clear");
  if (!voicebookClearArmed) {
    button.textContent = "Confirmer l'oubli";
    voicebookClearArmed = setTimeout(() => {
      voicebookClearArmed = null;
      button.textContent = "Tout oublier";
    }, 4000);
    return;
  }
  clearTimeout(voicebookClearArmed);
  voicebookClearArmed = null;
  button.textContent = "Tout oublier";
  try {
    await invoke("clear_voicebook");
    showVoicebookResult("Carnet de voix effacé.");
  } catch (e) {
    showVoicebookResult(String(e), true);
  } finally {
    await showVoicebook();
  }
});
window.__TAURI__.event.listen("voicebook", () => showVoicebook());

/* ---------- Rapport complet ---------- */

function currentReportTheme() {
  return window.matchMedia?.("(prefers-color-scheme: dark)")?.matches ? "dark" : "light";
}

function setMeetingTab(tab) {
  activeMeetingTab = tab === "report" ? "report" : "transcript";
  const transcriptBtn = $("meeting-tab-btn-transcript");
  const reportBtn = $("meeting-tab-btn-report");
  const transcriptPanel = $("meeting-panel-transcript");
  const reportPanel = $("meeting-panel-report");
  if (!transcriptBtn || !reportBtn || !transcriptPanel || !reportPanel) return;
  const isReport = activeMeetingTab === "report";
  transcriptBtn.setAttribute("aria-selected", String(!isReport));
  transcriptBtn.setAttribute("aria-pressed", String(!isReport));
  reportBtn.setAttribute("aria-selected", String(isReport));
  reportBtn.setAttribute("aria-pressed", String(isReport));
  transcriptPanel.hidden = isReport;
  reportPanel.hidden = !isReport;
  if (isReport && currentDetail?.record?.report) {
    loadReportFrame(currentDetail);
  }
}

function notifyReportPlaying(ms) {
  reportPlayingMs = ms;
  const frame = $("meeting-report-frame");
  try {
    frame?.contentWindow?.vttPlaying?.(ms);
  } catch {
    // Iframe pas encore prête.
  }
}

async function playReportClip(id, startMs) {
  const player = $("meeting-player");
  if (!player) return;
  try {
    player.src = await invoke("meeting_clip", { id, startMs });
    player.hidden = false;
    await player.play();
    notifyReportPlaying(startMs);
  } catch (e) {
    notifyReportPlaying(-1);
    showMeetingResult(String(e), true);
  }
}

function stopReportClip() {
  const player = $("meeting-player");
  if (player) player.pause();
  notifyReportPlaying(-1);
}

async function setReportActionDone(id, index, done) {
  try {
    await invoke("set_meeting_report_action_done", { id, index, done });
    if (currentDetail?.record?.id === id && currentDetail.record.report?.actions?.[index]) {
      currentDetail.record.report.actions[index].done = done;
      updateReportMeta(currentDetail.record.report);
    }
  } catch (e) {
    showMeetingResult(String(e), true);
  }
}

function updateReportMeta(report) {
  const meta = $("meeting-report-meta");
  if (!meta) return;
  if (!report) {
    meta.hidden = true;
    return;
  }
  const parts = [];
  if (report.at_ms) parts.push(meetingDateFormat.format(new Date(report.at_ms)));
  if (report.model) parts.push(report.model);
  if (report.cost != null) parts.push(formatCost(report.cost, report.currency));
  if (report.chapters?.length) parts.push(`${report.chapters.length} chapitre(s)`);
  if (report.actions?.length) {
    const done = report.actions.filter((a) => a.done).length;
    parts.push(`${done} / ${report.actions.length} action(s) faite(s)`);
  }
  meta.textContent = parts.join(" · ");
  meta.hidden = parts.length === 0;
}

function attachReportBridge(frame, recordId) {
  try {
    const win = frame.contentWindow;
    if (!win) return;
    win.VTT = {
      play: (ms) => playReportClip(recordId, ms),
      stop: () => stopReportClip(),
      setDone: (index, done) => setReportActionDone(recordId, index, done),
    };
    if (reportPlayingMs >= 0) win.vttPlaying?.(reportPlayingMs);
    if (pendingReportSearch) {
      const q = pendingReportSearch;
      pendingReportSearch = null;
      win.vttSearch?.(q);
    }
  } catch {
    // Environnement de test ou iframe non initialisée.
  }
}

async function loadReportFrame(detail) {
  const frame = $("meeting-report-frame");
  if (!frame || !detail?.record?.report) return;
  const { record } = detail;
  const theme = currentReportTheme();
  const namesKey = (detail.speakers || []).map((s) => s.name).join("|");
  const key = `${record.id}:${record.report.at_ms}:${detail.replay}:${theme}:${namesKey}`;
  if (key === reportFrameKey && !frame.hidden) {
    if (pendingReportSearch && typeof frame.contentWindow?.vttSearch === "function") {
      const q = pendingReportSearch;
      pendingReportSearch = null;
      try {
        frame.contentWindow.vttSearch(q);
      } catch {
        // Ignoré si l'iframe recharge.
      }
    }
    return;
  }
  try {
    const html = await invoke("meeting_report_html", { id: record.id, theme });
    if (selectedMeeting !== record.id) return;
    reportFrameKey = key;
    frame.onload = () => attachReportBridge(frame, record.id);
    frame.srcdoc = html;
    frame.hidden = false;
  } catch (e) {
    frame.hidden = true;
    showMeetingResult(String(e), true);
  }
}

function renderReportSection(detail) {
  const draft = $("meeting-report-draft");
  if (!draft) return;
  const { record } = detail;
  const report = record.report || null;
  draft.textContent = report ? "Refaire le rapport" : "Rédiger le rapport complet";
  draft.disabled = detail.busy || record.segments.length === 0 || !meetingsView?.llm_enabled;
  draft.title = meetingsView?.llm_enabled ? "" : "Nettoyage LLM désactivé dans les réglages";
  $("meeting-report-copy").hidden = !report;
  $("meeting-report-export").hidden = !report;
  $("meeting-report-empty").hidden = Boolean(report);
  updateReportMeta(report);
  const errorEl = $("meeting-report-error");
  errorEl.hidden = !record.report_error;
  errorEl.textContent = record.report_error ? `Rapport impossible : ${record.report_error}` : "";
  const frame = $("meeting-report-frame");
  if (!report) {
    reportFrameKey = "";
    frame.hidden = true;
    frame.srcdoc = "";
  } else if (activeMeetingTab === "report") {
    loadReportFrame(detail);
  }
}

for (const btn of document.querySelectorAll?.("[data-meeting-tab]") ?? []) {
  btn.addEventListener("click", () => setMeetingTab(btn.dataset.meetingTab));
}

$("meeting-report-draft")?.addEventListener("click", () =>
  meetingAction($("meeting-report-draft"), async () => {
    setMeetingTab("report");
    showMeetingResult("Rédaction du rapport complet par le LLM…");
    await invoke("draft_meeting_report", {
      id: selectedMeeting,
      language: $("meeting-summary-language").value,
    });
    reportFrameKey = "";
    showMeetingResult("✓ Rapport complet rédigé.");
  }),
);

$("meeting-report-copy")?.addEventListener("click", () =>
  meetingAction($("meeting-report-copy"), async () => {
    await invoke("copy_meeting", { id: selectedMeeting, part: "report" });
    showMeetingResult("✓ Rapport complet copié (Markdown).");
  }),
);

$("meeting-report-export")?.addEventListener("click", () =>
  meetingAction($("meeting-report-export"), async () => {
    const path = await invoke("export_meeting_report_html", { id: selectedMeeting });
    if (path) showMeetingResult(`✓ Rapport HTML exporté : ${path}`);
  }),
);

$("meeting-player")?.addEventListener("pause", () => notifyReportPlaying(-1));
$("meeting-player")?.addEventListener("ended", () => notifyReportPlaying(-1));

window.__TAURI__.event.listen("meeting-report-progress", (event) => {
  const payload = event?.payload;
  if (!payload || payload.id !== selectedMeeting || !isShown("reunions")) return;
  showMeetingResult(`Rédaction du rapport complet (étape ${payload.step} / ${payload.total})…`);
});

// Transcription au fil de l'eau : la page suit les blocs transcrits.
let meetingsRefresh = null;
window.__TAURI__.event.listen("meetings", () => {
  if (!isShown("reunions")) return;
  clearTimeout(meetingsRefresh);
  meetingsRefresh = setTimeout(showMeetings, 300);
});
window.__TAURI__.event.listen("meeting", (event) => {
  if (!isShown("reunions")) return;
  const payload = event?.payload;
  // Un décompte de fin d'appel ne change pas l'état recording : inutile de recharger la page.
  if (payload && Boolean(payload.recording) === Boolean(meetingsView?.recording)) return;
  showMeetings();
});
window.__TAURI__.event.listen("meeting-auto-stopped", (event) => {
  if (isShown("reunions") && event?.payload) showMeetingResult(String(event.payload));
});

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
