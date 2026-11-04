// Accueil : statistiques d'activité (compteurs par jour, sans texte).
const TYPING_WORDS_PER_MINUTE = 40;
const HOME_REFRESH_MS = 5000;
const dayFormat = new Intl.DateTimeFormat("fr-FR", { weekday: "short", day: "numeric", month: "short" });
const parseDay = (day) => new Date(`${day}T12:00:00`);

async function showUpdateBanner() {
  try {
    const status = await invoke("update_status");
    $("update-banner").hidden = !status.available;
    if (status.available) {
      $("update-text").textContent =
        `Nouvelle version ${status.available} disponible (installée : ${status.current}).`;
    }
  } catch {
    $("update-banner").hidden = true;
  }
}

$("update-install").addEventListener("click", async () => {
  $("update-install").disabled = true;
  $("update-text").textContent = "Téléchargement et vérification de la mise à jour…";
  try {
    await invoke("install_update");
  } catch (e) {
    $("update-text").textContent = String(e);
    $("update-install").disabled = false;
  }
});

window.__TAURI__.event.listen("update-progress", (event) => {
  if ($("update-install").disabled && event.payload !== null && event.payload < 100) {
    $("update-text").textContent = `Téléchargement de la mise à jour : ${event.payload} %`;
  }
});

async function showHome() {
  showUpdateBanner();
  let view;
  try {
    view = await invoke("home_view");
  } catch (e) {
    $("home-today").textContent = `Lecture impossible : ${e}`;
    return;
  }
  const { total, month, today } = view.summary;
  const minutesSpoken = total.audio_ms / 60000;
  const typingMs = (total.words / TYPING_WORDS_PER_MINUTE) * 60000;
  $("home-words").textContent = integer.format(total.words);
  $("home-saved").textContent = formatDuration(Math.max(0, typingMs - total.audio_ms));
  $("home-speed").textContent =
    minutesSpoken > 0 ? integer.format(Math.round(total.words / minutesSpoken)) : "–";
  $("home-cost").textContent = formatCost(month.cost, view.currency);
  $("home-today").textContent =
    `Aujourd'hui : ${integer.format(today.dictations)} dictée(s), ` +
    `${integer.format(today.words)} mots, ${formatCost(today.cost, view.currency)}.`;
  renderChart(view.days, view.currency);
}

function renderChart(days, currency) {
  const chart = $("home-chart");
  const max = Math.max(1, ...days.map((d) => d.totals.words));
  const tooltip = document.createElement("div");
  tooltip.className = "tooltip";
  tooltip.hidden = true;

  const bars = days.map(({ day, totals }) => {
    const bar = document.createElement("div");
    bar.className = "bar";
    bar.tabIndex = 0;
    const label =
      `${dayFormat.format(parseDay(day))} : ${integer.format(totals.words)} mots, ` +
      `${integer.format(totals.dictations)} dictée(s)`;
    bar.setAttribute("aria-label", label);
    const fill = document.createElement("span");
    fill.style.height = `${(totals.words / max) * 100}%`;
    bar.append(fill);
    const show = () => {
      tooltip.textContent = label;
      tooltip.hidden = false;
      // Centrée sur la barre, sans déborder du graphique.
      const half = tooltip.offsetWidth / 2;
      const center = bar.offsetLeft + bar.offsetWidth / 2;
      tooltip.style.left = `${Math.min(Math.max(center, half), chart.offsetWidth - half)}px`;
      tooltip.style.top = `${chart.offsetHeight - fill.offsetHeight}px`;
    };
    bar.addEventListener("mouseenter", show);
    bar.addEventListener("focus", show);
    bar.addEventListener("mouseleave", () => (tooltip.hidden = true));
    bar.addEventListener("blur", () => (tooltip.hidden = true));
    return bar;
  });
  chart.replaceChildren(...bars, tooltip);
  chart.setAttribute("aria-label", `Mots dictés par jour, maximum ${integer.format(max)}`);
  $("home-first-day").textContent = days.length ? dayFormat.format(parseDay(days[0].day)) : "";

  // Vue tableau (accessibilité, valeurs exactes), du plus récent au plus ancien.
  const rows = [...days].reverse().map(({ day, totals }) => {
    const row = document.createElement("tr");
    for (const value of [
      dayFormat.format(parseDay(day)),
      integer.format(totals.dictations),
      integer.format(totals.words),
      formatCost(totals.cost, currency),
    ]) {
      const cell = document.createElement(row.children.length ? "td" : "th");
      cell.textContent = value;
      row.append(cell);
    }
    return row;
  });
  $("home-table").tBodies[0].replaceChildren(...rows);
}

registerPage("accueil", { show: showHome });
setInterval(() => isShown("accueil") && showHome(), HOME_REFRESH_MS);
