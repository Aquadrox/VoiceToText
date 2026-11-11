// Menu principal repliable : icônes seules (libellé en bulle au survol) ou
// icônes et libellés. Replié d'office sur les pages qui ont leur propre menu
// (Réglages : attribut data-sub-nav), déplié ailleurs. Le bouton du bas
// inverse ce choix pour ce genre de page ; retenu d'une fois sur l'autre.
const MainNav = (() => {
  const KEY = "vtt-main-nav-compact";
  const DEFAULTS = { sub: true, page: false };
  const nav = $("main-nav");
  const toggle = $("nav-toggle");
  let prefs = { ...DEFAULTS };
  try {
    const saved = JSON.parse(localStorage.getItem(KEY) ?? "{}");
    prefs = { sub: saved.sub ?? DEFAULTS.sub, page: saved.page ?? DEFAULTS.page };
  } catch {
    // Pas de stockage ou valeur illisible : réglage par défaut.
  }

  const kind = () => ($(`page-${currentPage()}`)?.hasAttribute("data-sub-nav") ? "sub" : "page");

  function update() {
    const compact = Boolean(prefs[kind()]);
    nav.classList.toggle("compact", compact);
    toggle.querySelector(".label").textContent = compact ? "Déplier le menu" : "Réduire le menu";
    toggle.querySelector("use").setAttribute("href", compact ? "#i-panel-left-open" : "#i-panel-left-close");
  }

  toggle.addEventListener("click", () => {
    prefs[kind()] = !nav.classList.contains("compact");
    try {
      localStorage.setItem(KEY, JSON.stringify(prefs));
    } catch {
      // Pas de stockage : le choix vaut jusqu'à la fermeture de la fenêtre.
    }
    update();
  });

  // Animation seulement après le premier affichage (pas de repli visible à l'ouverture).
  requestAnimationFrame(() => requestAnimationFrame(() => nav.classList.add("animated")));

  return { update };
})();

// Navigation entre les pages (ancre de l'URL : #accueil, #historique,
// #reglages/nettoyage…).
function route() {
  const name = PAGES[currentPage()] ? currentPage() : "accueil";
  for (const section of document.querySelectorAll(".page")) {
    section.hidden = section.id !== `page-${name}`;
  }
  for (const link of document.querySelectorAll("nav.main-nav a")) {
    if (link.hash === `#${name}`) link.setAttribute("aria-current", "page");
    else link.removeAttribute("aria-current");
  }
  MainNav.update();
  PAGES[name].show();
}

window.addEventListener("hashchange", route);
// Retour sur la fenêtre (après des dictées ailleurs) : données à jour.
window.addEventListener("focus", () => PAGES[currentPage()]?.show());
route();
