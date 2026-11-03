// Navigation entre les pages (ancre de l'URL : #accueil, #historique, #reglages).
function route() {
  const name = PAGES[currentPage()] ? currentPage() : "accueil";
  for (const section of document.querySelectorAll(".page")) {
    section.hidden = section.id !== `page-${name}`;
  }
  for (const link of document.querySelectorAll("nav a")) {
    if (link.hash === `#${name}`) link.setAttribute("aria-current", "page");
    else link.removeAttribute("aria-current");
  }
  PAGES[name].show();
}

window.addEventListener("hashchange", route);
// Retour sur la fenêtre (après des dictées ailleurs) : données à jour.
window.addEventListener("focus", () => PAGES[currentPage()]?.show());
route();
