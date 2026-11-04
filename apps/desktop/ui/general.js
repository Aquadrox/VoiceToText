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
    await invoke("save_app_settings", {
      hotkey: {
        key: $("hk-key").value,
        command_key: $("hk-command").value,
        paste_raw: $("hk-paste").value.trim(),
        retry: $("hk-retry").value.trim(),
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
    });
    showGeneralResult("Enregistré. Redémarrage de VoiceToText…");
  } catch (e) {
    showGeneralResult(String(e), true);
    button.disabled = false;
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
