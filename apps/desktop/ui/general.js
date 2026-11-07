// Réglages généraux : raccourcis, reconnaissance locale, dictée, capture
// commentée, application. Appliqués section par section (settings-ui.js).
let generalFilled = false;

/** Libellés affichés (la configuration garde les noms techniques). */
const KEY_LABELS = {
  RightCtrl: "Ctrl droit",
  RightShift: "Maj droite",
  CapsLock: "Verr. Maj",
  ScrollLock: "Arrêt défil.",
  Insert: "Inser",
};
const keyLabel = (key) => KEY_LABELS[key] ?? key;

function option(value, label) {
  const element = document.createElement("option");
  element.value = value;
  element.textContent = label;
  return element;
}

/** Moteurs de transcription, dans l'ordre de la liste. */
const ENGINES = [
  {
    id: "whisper",
    label: "Whisper (OpenAI) : le plus précis, idéal avec une carte graphique",
    note: "Whisper : le plus précis. Avec une carte graphique, quasi instantané ; sur le processeur seul, préférez Parakeet.",
  },
  {
    id: "parakeet",
    label: "Parakeet (NVIDIA) : rapide sur le processeur",
    note: "Parakeet : pensé pour le processeur, rapide et ponctué, langue détectée automatiquement. Le dictionnaire ne sert qu'au nettoyage LLM. Télécharge aussi ONNX Runtime (Microsoft).",
  },
  {
    id: "vosk",
    label: "Vosk : très léger, pour les vieux PC",
    note: "Vosk : très léger (quelques dizaines de Mo), moins précis, sans ponctuation ni majuscules : gardez le nettoyage LLM activé. Le dictionnaire ne sert qu'au nettoyage LLM. Télécharge aussi la bibliothèque Vosk.",
  },
];
/** Moteur de chaque modèle proposé. */
let engineOf = {};

function showEngineNote() {
  const engine = engineOf[$("asr-model").value] ?? "whisper";
  $("asr-engine-note").textContent = ENGINES.find((e) => e.id === engine)?.note ?? "";
  $("asr-gpu").disabled = engine !== "whisper";
}

$("asr-model").addEventListener("change", showEngineNote);

function fillGeneral(view) {
  const keys = view.keys.map((k) => option(k, keyLabel(k)));
  $("hk-key").replaceChildren(...keys);
  $("hk-key").value = view.hotkey.key;
  $("hk-command").replaceChildren(
    option("", "Désactivé"),
    ...view.keys.map((k) => option(k, keyLabel(k))),
  );
  $("hk-command").value = view.hotkey.command_key;
  $("hk-paste").value = view.hotkey.paste_raw;
  $("hk-retry").value = view.hotkey.retry;
  $("hk-translate").value = view.hotkey.translate;
  $("hk-meeting").value = view.hotkey.meeting;
  $("hk-meeting-copy").value = view.hotkey.meeting_copy;
  $("hk-annotate").value = view.hotkey.annotate;
  $("capture-template").replaceChildren(...view.capture_templates.map(([id, label]) => option(id, label)));
  $("capture-template").value = view.captures.template;
  $("capture-keep").replaceChildren(
    ...view.capture_keep_days.map((d) => option(String(d), d === 0 ? "Non" : d === 1 ? "1 jour" : `${d} jours`)),
  );
  $("capture-keep").value = String(view.captures.keep_days);
  $("capture-area").value = view.captures.area;
  $("capture-clicks").checked = view.captures.clicks;
  $("capture-vision").checked = view.captures.vision;
  const profiles = view.llm_profiles.includes(view.captures.vision_profile) || !view.captures.vision_profile
    ? view.llm_profiles
    : [...view.llm_profiles, view.captures.vision_profile];
  $("capture-vision-profile").replaceChildren(
    option("", "Fournisseur actif (section Nettoyage par LLM)"),
    ...profiles.map((name) => option(name, `Profil « ${name} »`)),
  );
  $("capture-vision-profile").value = view.captures.vision_profile;
  showVisionWarning();
  $("hk-hold").value = view.hotkey.min_hold_ms;
  $("hk-handsfree").checked = view.hotkey.hands_free;

  const microphones = view.microphones.map((name) => option(name, name));
  const configured = view.audio.device;
  if (configured && !view.microphones.includes(configured)) {
    microphones.unshift(option(configured, `${configured} (introuvable : micro par défaut utilisé)`));
  }
  $("audio-device").replaceChildren(option("", "Micro par défaut de Windows"), ...microphones);
  $("audio-device").value = configured;

  // Un groupe par moteur ; un modèle placé à la main reste proposé.
  const groups = ENGINES.map(({ id, label }) => {
    const group = document.createElement("optgroup");
    group.label = label;
    group.append(
      ...view.models
        .filter((m) => m.engine === id)
        .map((m) =>
          option(
            m.file,
            `${m.label} · ${integer.format(m.size_mb)} Mo${m.downloaded ? " · téléchargé" : ""}`,
          ),
        ),
    );
    return group;
  });
  if (!view.models.some((m) => m.file === view.asr.model)) {
    groups[0].append(option(view.asr.model, `${view.asr.model} (placé à la main)`));
  }
  engineOf = Object.fromEntries(view.models.map((m) => [m.file, m.engine]));
  $("asr-model").replaceChildren(...groups);
  $("asr-model").value = view.asr.model;
  showEngineNote();
  $("asr-gpu").checked = view.asr.use_gpu;
  $("asr-unload").replaceChildren(
    ...view.unload_choices.map((min) => option(String(min), min === 0 ? "Jamais" : `${min} min`)),
  );
  $("asr-unload").value = String(view.asr.unload_after_min);
  $("asr-fullscreen").checked = view.asr.unload_in_fullscreen;
  $("updates-check").checked = view.updates.check;
  $("dict-sounds").checked = view.dictation.sounds;
  $("dict-media").checked = view.dictation.pause_media;
  $("dict-commands").checked = view.dictation.voice_commands;

  // Une langue saisie à la main dans config.toml reste proposée.
  const languages = view.languages.includes(view.translation.language)
    ? view.languages
    : [...view.languages, view.translation.language];
  $("tr-language").replaceChildren(...languages.map((l) => option(l, l)));
  $("tr-language").value = view.translation.language;
}

/** Réglages généraux appliqués (zone de test, raccourcis cités dans les sections). */
let generalView = null;

/** Raccourcis cités dans les sections (« Raccourci Alt+Shift+T »). */
function showShortcuts() {
  for (const element of document.querySelectorAll("[data-shortcut]")) {
    const control = $(element.dataset.shortcut);
    const value = control ? SettingsUI.valueFor(control, "-") : "";
    element.textContent = value || "(désactivé, voir Raccourcis)";
  }
}

/** Remplit les sections générales ; les saisies en attente de `keep` sont gardées. */
function fillAndKeep(view, keep = []) {
  const pending = SettingsUI.pendingValues(keep);
  fillGeneral(view);
  generalView = view;
  for (const name of SettingsUI.panelsOfKind("general")) SettingsUI.markClean(name);
  SettingsUI.restorePending(pending);
  showEngineNote();
  showVisionWarning();
  for (const name of SettingsUI.panelsOfKind("general")) SettingsUI.refresh(name);
  showShortcuts();
  window.dispatchEvent(new Event("settings-applied"));
}

async function showGeneral() {
  try {
    const view = await invoke("app_settings_view");
    // Autostart : toujours l'état réel ; le formulaire, une seule fois.
    $("autostart").checked = view.autostart;
    if (!generalFilled) fillAndKeep(view);
    generalFilled = true;
    const status = await invoke("update_status");
    $("current-version").textContent = status.current;
  } catch (e) {
    SettingsUI.showResult("raccourcis", String(e), true);
  }
}

/** Réglages généraux à envoyer : seule la section `applying` est prise du formulaire. */
function readGeneral(applying) {
  const v = (id) => SettingsUI.valueFor($(id), applying);
  return {
    hotkey: {
      key: v("hk-key"),
      command_key: v("hk-command"),
      paste_raw: v("hk-paste").trim(),
      retry: v("hk-retry").trim(),
      translate: v("hk-translate").trim(),
      meeting: v("hk-meeting").trim(),
      meeting_copy: v("hk-meeting-copy").trim(),
      annotate: v("hk-annotate").trim(),
      min_hold_ms: Number(v("hk-hold")),
      hands_free: v("hk-handsfree"),
    },
    audio: { device: v("audio-device") },
    asr: {
      model: v("asr-model"),
      use_gpu: v("asr-gpu"),
      unload_after_min: Number(v("asr-unload")),
      unload_in_fullscreen: v("asr-fullscreen"),
    },
    updates: { check: v("updates-check") },
    dictation: {
      sounds: v("dict-sounds"),
      pause_media: v("dict-media"),
      voice_commands: v("dict-commands"),
    },
    translation: { language: v("tr-language") },
    captures: {
      template: v("capture-template"),
      keep_days: Number(v("capture-keep")),
      area: v("capture-area"),
      clicks: v("capture-clicks"),
      vision: v("capture-vision"),
      vision_profile: v("capture-vision-profile"),
    },
  };
}

for (const name of SettingsUI.panelsOfKind("general")) {
  SettingsUI.define(name, {
    apply: async (applying) => {
      const result = await invoke("save_app_settings", readGeneral(applying));
      // Listes à jour (modèle téléchargé…) ; les autres sections gardent leurs saisies.
      const view = await invoke("app_settings_view");
      const others = SettingsUI.panelsOfKind("general").filter((n) => n !== applying);
      fillAndKeep(view, others);
      const notes = result.notes.map((note) => `\n• ${note}`).join("");
      return `✓ Appliqué, sans redémarrage : essayez dans la zone de test ci-dessous.${notes}`;
    },
    revert: () => {
      const others = SettingsUI.panelsOfKind("general").filter((n) => n !== name);
      if (generalView) fillAndKeep(generalView, others);
    },
  });
}

// Mises à jour : vérification à la demande, puis installation sur clic.
function showUpdateResult(text, isError = false) {
  const result = $("settings-update-result");
  result.hidden = false;
  result.textContent = text;
  result.classList.toggle("error", isError);
}

$("settings-update-check").addEventListener("click", async () => {
  const button = $("settings-update-check");
  button.disabled = true;
  $("settings-update-install").hidden = true;
  showUpdateResult("Recherche d'une nouvelle version…");
  try {
    const status = await invoke("check_update");
    if (!status.configured) {
      showUpdateResult("Mises à jour non configurées dans cette version.", true);
    } else if (status.available) {
      showUpdateResult(`Version ${status.available} disponible (installée : ${status.current}).`);
      $("settings-update-install").hidden = false;
    } else {
      showUpdateResult(`✓ VoiceToText est à jour (version ${status.current}).`);
    }
  } catch (e) {
    showUpdateResult(String(e), true);
  } finally {
    button.disabled = false;
  }
});

$("settings-update-install").addEventListener("click", async () => {
  const install = $("settings-update-install");
  install.disabled = true;
  $("settings-update-check").disabled = true;
  showUpdateResult("Téléchargement de la mise à jour…");
  try {
    // VoiceToText se ferme pendant l'installation puis redémarre.
    await invoke("install_update");
  } catch (e) {
    showUpdateResult(String(e), true);
    install.disabled = false;
    $("settings-update-check").disabled = false;
  }
});

window.__TAURI__.event.listen("update-progress", (event) => {
  if ($("settings-update-install").disabled && event.payload !== null) {
    showUpdateResult(
      event.payload < 100
        ? `Téléchargement de la mise à jour : ${event.payload} %`
        : "Téléchargé : vérification de la signature puis installation (VoiceToText va redémarrer)…",
    );
  }
});

$("autostart").addEventListener("change", async () => {
  const checkbox = $("autostart");
  try {
    await invoke("set_autostart", { enabled: checkbox.checked });
    $("autostart-status").textContent = checkbox.checked
      ? "✓ VoiceToText démarrera avec Windows."
      : "VoiceToText ne démarrera plus avec Windows.";
  } catch (e) {
    checkbox.checked = !checkbox.checked;
    $("autostart-status").textContent = String(e);
  }
});

/* ---------- Export / import des réglages ---------- */

const IMPORT_MESSAGE_KEY = "vtt-import-message";

function showTransferResult(text, isError = false) {
  const result = $("transfer-result");
  result.hidden = false;
  result.textContent = text;
  result.classList.toggle("error", isError);
}

$("settings-export").addEventListener("click", async () => {
  const button = $("settings-export");
  button.disabled = true;
  try {
    const exported = await invoke("export_settings");
    if (!exported) return;
    const removed = exported.removed.length
      ? `\nRetirés par précaution (ressemblent à un secret) : ${exported.removed.join(", ")}.`
      : "";
    showTransferResult(`✓ Réglages exportés, sans les clés API : ${exported.path}${removed}`);
  } catch (e) {
    showTransferResult(String(e), true);
  } finally {
    button.disabled = false;
  }
});

$("settings-import").addEventListener("click", async () => {
  const button = $("settings-import");
  button.disabled = true;
  try {
    const imported = await invoke("import_settings");
    if (!imported) return;
    const backup = imported.backup ? `\nAnciens réglages gardés dans ${imported.backup}.` : "";
    const notes = imported.notes.map((note) => `\n• ${note}`).join("");
    const message =
      `✓ Réglages importés et appliqués, sans redémarrage. Les clés API ne sont pas dans le fichier : celles déjà enregistrées sur ce PC restent utilisées.` +
      `${backup}${notes}`;
    // Toutes les pages affichent les nouveaux réglages : rechargement.
    try {
      sessionStorage.setItem(IMPORT_MESSAGE_KEY, message);
      location.reload();
    } catch {
      showTransferResult(message);
    }
  } catch (e) {
    showTransferResult(String(e), true);
  } finally {
    button.disabled = false;
  }
});

try {
  const message = sessionStorage.getItem(IMPORT_MESSAGE_KEY);
  if (message) {
    sessionStorage.removeItem(IMPORT_MESSAGE_KEY);
    showTransferResult(message);
  }
} catch {
  // Pas de stockage : rien à afficher.
}

function showVisionWarning() {
  const vision = $("capture-vision").checked;
  $("capture-vision-warning").hidden = !vision;
  $("capture-vision-profile").disabled = !vision;
}
$("capture-vision").addEventListener("change", showVisionWarning);
