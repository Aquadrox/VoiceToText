// Page Prompts : prompts système personnalisés, prompt actif, test.
const KINDS = [
  {
    kind: "nettoyage",
    active: "cleanup_active",
    title: "Nettoyage des dictées",
    note: "Le texte dicté arrive entre <dictee> et </dictee> ; la réponse doit être le texte final seul.",
    defaultKey: "cleanup",
  },
  {
    kind: "commande",
    active: "command_active",
    title: "Command Mode",
    note: "La consigne arrive entre <consigne> et </consigne>, le texte sélectionné entre <selection> et </selection> ; la réponse remplace la sélection.",
    defaultKey: "command",
  },
];
let promptsState = null;
let promptsDirty = false;

function promptsChanged() {
  promptsDirty = true;
  const status = $("prompts-status");
  status.textContent = "Modifications non enregistrées";
  status.className = "small dirty";
}

function showTest(area, test) {
  area.hidden = false;
  area.classList.toggle("error", !test.ok);
  const lines = [`Exemple : ${test.input}`, ""];
  if (test.ok) {
    lines.push(`Réponse (${test.duration_ms} ms) :`, test.output);
    if (test.usage) {
      lines.push(
        "",
        `${test.usage.input_tokens} + ${test.usage.output_tokens} jetons · ${formatCost(test.cost ?? 0, test.currency)}`,
      );
    }
  } else {
    lines.push(`Échec : ${test.error}`);
  }
  area.textContent = lines.join("\n");
}

async function runPromptTest(kind, text, button, area) {
  button.disabled = true;
  area.hidden = false;
  area.classList.remove("error");
  area.textContent = "Test en cours (fournisseur LLM actif)…";
  try {
    showTest(area, await invoke("test_prompt", { kind, text }));
  } catch (e) {
    area.classList.add("error");
    area.textContent = String(e);
  } finally {
    button.disabled = false;
  }
}

function uniqueName(kind, base) {
  const taken = new Set(promptsState.prompts.filter((p) => p.kind === kind).map((p) => p.name.toLowerCase()));
  let name = base;
  for (let n = 2; taken.has(name.toLowerCase()); n++) name = `${base} ${n}`;
  return name;
}

function renderPrompts() {
  const blocks = KINDS.map((k) => {
    const block = $("prompt-kind").content.firstElementChild.cloneNode(true);
    block.querySelector(".kind-title").textContent = k.title;
    block.querySelector(".kind-note").textContent = k.note;
    const defaultText = promptsState.defaults[k.defaultKey];
    block.querySelector(".default").value = defaultText;

    const select = block.querySelector(".active");
    const mine = promptsState.prompts.filter((p) => p.kind === k.kind);
    select.replaceChildren(option("", "Prompt intégré"), ...mine.map((p) => option(p.name, p.name)));
    select.value = promptsState[k.active];
    select.addEventListener("change", () => {
      promptsState[k.active] = select.value;
      promptsChanged();
    });

    const defaultArea = block.querySelector("details .test-result");
    const testDefault = block.querySelector(".test-default");
    testDefault.addEventListener("click", () => runPromptTest(k.kind, defaultText, testDefault, defaultArea));
    block.querySelector(".copy-default").addEventListener("click", () => {
      promptsState.prompts.push({ name: uniqueName(k.kind, "Mon prompt"), kind: k.kind, text: defaultText });
      promptsChanged();
      renderPrompts();
    });

    const cards = mine.map((prompt) => {
      const card = $("prompt-card").content.firstElementChild.cloneNode(true);
      const name = card.querySelector(".name");
      const text = card.querySelector(".text");
      name.value = prompt.name;
      text.value = prompt.text;
      name.addEventListener("change", () => {
        // Le prompt actif suit son renommage.
        if (promptsState[k.active] === prompt.name) promptsState[k.active] = name.value.trim();
        prompt.name = name.value.trim();
        promptsChanged();
        renderPrompts();
      });
      text.addEventListener("input", () => {
        prompt.text = text.value;
        promptsChanged();
      });
      const test = card.querySelector(".test");
      const area = card.querySelector(".test-result");
      test.addEventListener("click", () => runPromptTest(k.kind, text.value, test, area));
      card.querySelector(".remove").addEventListener("click", () => {
        promptsState.prompts.splice(promptsState.prompts.indexOf(prompt), 1);
        if (promptsState[k.active] === prompt.name) promptsState[k.active] = "";
        promptsChanged();
        renderPrompts();
      });
      return card;
    });
    block.querySelector(".customs").replaceChildren(...cards);
    return block;
  });
  $("prompt-kinds").replaceChildren(...blocks);
}

$("prompts-save").addEventListener("click", async () => {
  const button = $("prompts-save");
  const status = $("prompts-status");
  button.disabled = true;
  try {
    await invoke("save_prompts", {
      prompts: promptsState.prompts,
      cleanupActive: promptsState.cleanup_active,
      commandActive: promptsState.command_active,
    });
    promptsDirty = false;
    status.className = "small muted";
    status.textContent = "Enregistré ✓ appliqué immédiatement";
    onPromptsSaved(promptsState.cleanup_active, promptsState.command_active);
  } catch (e) {
    status.className = "small error";
    status.textContent = String(e);
  } finally {
    button.disabled = false;
  }
});

registerPage("prompts", {
  show: async () => {
    if (promptsDirty && promptsState) return;
    try {
      promptsState = await invoke("prompts_view");
      renderPrompts();
    } catch (e) {
      $("prompts-status").textContent = String(e);
    }
  },
});
