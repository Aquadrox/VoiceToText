// Pages Dictionnaire, Snippets et Styles : édition puis enregistrement dans
// config.toml, appliqué sans redémarrer.
const vocabulary = { dictionary: [], snippets: [], styles: [] };
/** Sections modifiées et pas encore enregistrées (non rechargées au retour). */
const dirty = { dictionary: false, snippets: false, styles: false };

const STYLE_TEMPLATES = {
  messagerie: {
    apps: ["slack.exe", "discord.exe", "ms-teams.exe", "telegram.exe", "signal.exe"],
    titles: ["WhatsApp", "Messenger"],
    instructions: "Messagerie : ton conversationnel, phrases courtes, pas de formule de politesse.",
    cleanup: true,
  },
  email: {
    apps: ["outlook.exe", "olk.exe", "thunderbird.exe"],
    titles: ["Gmail", "Outlook"],
    instructions: "E-mail : ton soigné, paragraphes aérés.",
    cleanup: true,
  },
  code: {
    apps: ["code.exe", "cursor.exe", "idea64.exe", "devenv.exe"],
    titles: [],
    instructions: "",
    cleanup: false,
  },
};

async function loadVocabulary() {
  const view = await invoke("vocabulary_view");
  for (const section of Object.keys(vocabulary)) {
    if (!dirty[section]) vocabulary[section] = view[section];
  }
}

function setStatus(section, text, kind = "") {
  const status = $(`${section}-status`);
  status.textContent = text;
  status.className = `small ${kind || "muted"}`;
}

function markDirty(section, prefix) {
  dirty[section] = true;
  setStatus(prefix, "Modifications non enregistrées", "dirty");
}

async function save(section, prefix, button) {
  button.disabled = true;
  try {
    await invoke("save_vocabulary", { update: { [section]: vocabulary[section] } });
    dirty[section] = false;
    setStatus(prefix, "Enregistré ✓ appliqué immédiatement");
    await loadVocabulary();
    return true;
  } catch (e) {
    setStatus(prefix, String(e), "error");
    return false;
  } finally {
    button.disabled = false;
  }
}

const splitList = (text) =>
  text
    .split(",")
    .map((item) => item.trim())
    .filter(Boolean);

/* ---------- Dictionnaire ---------- */

function renderDictionary() {
  const items = vocabulary.dictionary.map((word, index) => {
    const item = document.createElement("li");
    const label = document.createElement("span");
    label.textContent = word;
    const remove = document.createElement("button");
    remove.textContent = "×";
    remove.title = `Retirer « ${word} »`;
    remove.addEventListener("click", () => {
      vocabulary.dictionary.splice(index, 1);
      markDirty("dictionary", "dict");
      renderDictionary();
    });
    item.append(label, remove);
    return item;
  });
  $("dict-list").replaceChildren(...items);
  $("dict-empty").hidden = items.length > 0;
}

$("dict-form").addEventListener("submit", (event) => {
  event.preventDefault();
  const word = $("dict-input").value.trim();
  if (!word) return;
  const exists = vocabulary.dictionary.some((w) => w.toLowerCase() === word.toLowerCase());
  if (!exists) {
    vocabulary.dictionary.push(word);
    markDirty("dictionary", "dict");
  }
  $("dict-input").value = "";
  renderDictionary();
});

$("dict-save").addEventListener("click", async () => {
  if (await save("dictionary", "dict", $("dict-save"))) renderDictionary();
});

registerPage("dictionnaire", {
  show: async () => {
    try {
      await loadVocabulary();
    } catch (e) {
      setStatus("dict", String(e), "error");
    }
    renderDictionary();
  },
});

/* ---------- Snippets ---------- */

function renderSnippets() {
  const cards = vocabulary.snippets.map((snippet, index) => {
    const card = $("snippet-card").content.firstElementChild.cloneNode(true);
    const trigger = card.querySelector(".trigger");
    const text = card.querySelector(".text");
    trigger.value = snippet.trigger;
    text.value = snippet.text;
    trigger.addEventListener("input", () => {
      snippet.trigger = trigger.value;
      markDirty("snippets", "snippet");
    });
    text.addEventListener("input", () => {
      snippet.text = text.value;
      markDirty("snippets", "snippet");
    });
    card.querySelector(".remove").addEventListener("click", () => {
      vocabulary.snippets.splice(index, 1);
      markDirty("snippets", "snippet");
      renderSnippets();
    });
    return card;
  });
  $("snippet-list").replaceChildren(...cards);
}

$("snippet-add").addEventListener("click", () => {
  vocabulary.snippets.push({ trigger: "", text: "" });
  markDirty("snippets", "snippet");
  renderSnippets();
  $("snippet-list").lastElementChild?.querySelector(".trigger").focus();
});

$("snippet-save").addEventListener("click", async () => {
  if (await save("snippets", "snippet", $("snippet-save"))) renderSnippets();
});

registerPage("snippets", {
  show: async () => {
    try {
      await loadVocabulary();
    } catch (e) {
      setStatus("snippet", String(e), "error");
    }
    renderSnippets();
  },
});

/* ---------- Styles ---------- */

function renderStyles() {
  const cards = vocabulary.styles.map((style, index) => {
    const card = $("style-card").content.firstElementChild.cloneNode(true);
    card.querySelector(".style-name").textContent = `Style n°${index + 1}`;
    const apps = card.querySelector(".apps");
    const titles = card.querySelector(".titles");
    const cleanup = card.querySelector(".cleanup");
    const instructions = card.querySelector(".instructions");
    apps.value = style.apps.join(", ");
    titles.value = style.titles.join(", ");
    cleanup.checked = style.cleanup;
    instructions.value = style.instructions;
    instructions.disabled = !style.cleanup;

    const changed = () => markDirty("styles", "style");
    apps.addEventListener("input", () => {
      style.apps = splitList(apps.value);
      changed();
    });
    titles.addEventListener("input", () => {
      style.titles = splitList(titles.value);
      changed();
    });
    cleanup.addEventListener("change", () => {
      style.cleanup = cleanup.checked;
      instructions.disabled = !cleanup.checked;
      changed();
    });
    instructions.addEventListener("input", () => {
      style.instructions = instructions.value;
      changed();
    });

    const up = card.querySelector(".up");
    up.disabled = index === 0;
    up.addEventListener("click", () => {
      const [moved] = vocabulary.styles.splice(index, 1);
      vocabulary.styles.splice(index - 1, 0, moved);
      changed();
      renderStyles();
    });
    card.querySelector(".remove").addEventListener("click", () => {
      vocabulary.styles.splice(index, 1);
      changed();
      renderStyles();
    });
    return card;
  });
  $("style-list").replaceChildren(...cards);
}

function addStyle(style) {
  vocabulary.styles.push(structuredClone(style));
  markDirty("styles", "style");
  renderStyles();
  $("style-list").lastElementChild?.scrollIntoView({ block: "nearest" });
}

$("style-add").addEventListener("click", () =>
  addStyle({ apps: [], titles: [], instructions: "", cleanup: true }),
);
for (const button of document.querySelectorAll("[data-template]")) {
  button.addEventListener("click", () => addStyle(STYLE_TEMPLATES[button.dataset.template]));
}

$("style-save").addEventListener("click", async () => {
  if (await save("styles", "style", $("style-save"))) renderStyles();
});

registerPage("styles", {
  show: async () => {
    try {
      await loadVocabulary();
    } catch (e) {
      setStatus("style", String(e), "error");
    }
    renderStyles();
  },
});
