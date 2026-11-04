// Réglages généraux : raccourcis, micro, modèle Whisper, démarrage avec Windows.
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
  $("hk-hold").value = view.hotkey.min_hold_ms;
  $("hk-handsfree").checked = view.hotkey.hands_free;

  const microphones = view.microphones.map((name) => option(name, name));
  const configured = view.audio.device;
  if (configured && !view.microphones.includes(configured)) {
    microphones.unshift(option(configured, `${configured} (introuvable : micro par défaut utilisé)`));
  }
  $("audio-device").replaceChildren(option("", "Micro par défaut de Windows"), ...microphones);
  $("audio-device").value = configured;

  $("asr-model").replaceChildren(
    ...view.models.map((m) =>
      option(
        m.file,
        `${m.label} · ${integer.format(m.size_mb)} Mo${m.downloaded ? " · téléchargé" : ""}`,
      ),
    ),
  );
  $("asr-model").value = view.asr.model;
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

async function showGeneral() {
  try {
    const view = await invoke("app_settings_view");
    // Autostart : toujours l'état réel ; le formulaire, une seule fois.
    $("autostart").checked = view.autostart;
    if (!generalFilled) fillGeneral(view);
    generalFilled = true;
    const status = await invoke("update_status");
    $("current-version").textContent = status.current;
  } catch (e) {
    showGeneralResult(String(e), true);
  }
}

function showGeneralResult(text, isError = false) {
  const result = $("general-result");
  result.hidden = false;
  result.textContent = text;
  result.classList.toggle("error", isError);
}

$("general-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const button = $("general-save");
  button.disabled = true;
  try {
    const result = await invoke("save_app_settings", {
      hotkey: {
        key: $("hk-key").value,
        command_key: $("hk-command").value,
        paste_raw: $("hk-paste").value.trim(),
        retry: $("hk-retry").value.trim(),
        translate: $("hk-translate").value.trim(),
        min_hold_ms: Number($("hk-hold").value),
        hands_free: $("hk-handsfree").checked,
      },
      audio: { device: $("audio-device").value },
      asr: {
        model: $("asr-model").value,
        use_gpu: $("asr-gpu").checked,
        unload_after_min: Number($("asr-unload").value),
        unload_in_fullscreen: $("asr-fullscreen").checked,
      },
      updates: { check: $("updates-check").checked },
      dictation: {
        sounds: $("dict-sounds").checked,
        pause_media: $("dict-media").checked,
        voice_commands: $("dict-commands").checked,
      },
      translation: { language: $("tr-language").value },
    });
    const notes = result.notes.map((note) => `\n• ${note}`).join("");
    showGeneralResult(`✓ Appliqué, sans redémarrage.${notes}`);
    generalFilled = false;
    showGeneral();
  } catch (e) {
    showGeneralResult(String(e), true);
  } finally {
    button.disabled = false;
  }
});

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
