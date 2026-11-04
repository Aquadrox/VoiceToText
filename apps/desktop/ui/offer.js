// Appel détecté : enregistrer, plus tard, ou retenir un choix pour ce service.
const { invoke } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);

async function refresh() {
  const offer = await invoke("offer_view");
  if (!offer) return;
  $("title").textContent = offer.browser ? `Appel dans ${offer.label} ?` : `Appel ${offer.label} détecté`;
  showMore(false);
}

function showMore(more) {
  $("main-actions").hidden = more;
  $("more-actions").hidden = !more;
  $("more").setAttribute("aria-expanded", String(more));
}

async function answer(choice) {
  for (const button of document.querySelectorAll("button")) button.disabled = true;
  try {
    await invoke("offer_answer", { choice });
  } catch (e) {
    $("title").textContent = String(e);
    for (const button of document.querySelectorAll("button")) button.disabled = false;
  }
}

$("record").addEventListener("click", () => answer("enregistrer"));
$("later").addEventListener("click", () => answer("plus-tard"));
$("always").addEventListener("click", () => answer("toujours"));
$("never").addEventListener("click", () => answer("jamais"));
$("more").addEventListener("click", () => showMore(true));
$("back").addEventListener("click", () => showMore(false));
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") answer("plus-tard");
});
window.__TAURI__.event.listen("offer", refresh);
refresh();
