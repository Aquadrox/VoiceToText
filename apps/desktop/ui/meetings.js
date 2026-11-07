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
    when.textContent = `${kind}${meetingDateFormat.format(new Date(meeting.started_at_ms))} · ${status}`;
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
  if (article.dataset.id !== record.id) {
    // Autre enregistrement : le lecteur repart de zéro.
    article.dataset.id = record.id;
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

/** Intervenants distingués : nom à donner, temps de parole, première phrase. */
function renderSpeakers(detail) {
  const { record, speakers } = detail;
  $("meeting-speakers-block").hidden = speakers.length === 0;
  if (speakers.length === 0) return;
  const list = $("meeting-speakers");
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
    where.textContent =
      `${result.title} · ${meetingDateFormat.format(new Date(result.started_at_ms))} · ${clock(result.start_ms)}`;
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
      highlightAt = result.start_ms;
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
    const cost = result.cost != null ? ` · ${formatCost(result.cost, result.currency)}` : "";
    answer.textContent = `${result.text}\n\n(${result.sources} enregistrement(s) consulté(s)${cost})`;
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
        echo_filter: $("meeting-echo").checked,
        summary_language: $("meeting-language").value,
        diarize: $("meeting-diarize").checked,
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
    return "✓ Appliqué (l'audio et le son du PC : dès la prochaine réunion).";
  },
});

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
