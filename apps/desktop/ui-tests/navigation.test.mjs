import test from "node:test";
import assert from "node:assert/strict";
import { createEnvironment } from "./dom.mjs";

function setupNav(opts = {}) {
  const env = createEnvironment(opts);
  const { el } = env;

  const toggleLabel = el("span", { className: "label" });
  const toggleUse = el("use");
  const toggle = el("button", { id: "nav-toggle" }, toggleLabel, toggleUse);
  const linkHome = el("a", { attrs: { href: "#accueil" } });
  const linkSettings = el("a", { attrs: { href: "#reglages" } });
  const nav = el("nav", { id: "main-nav", className: "main-nav" }, linkHome, linkSettings, toggle);

  const pageHome = el("section", { id: "page-accueil", className: "page" });
  const pageSettings = el("section", {
    id: "page-reglages",
    className: "page",
    attrs: { "data-sub-nav": "" },
  });

  env.document.body.append(nav, pageHome, pageSettings);
  env.load("common.js");

  const shown = [];
  env.eval("registerPage")("accueil", { show: () => shown.push("accueil") });
  env.eval("registerPage")("reglages", { show: () => shown.push("reglages") });
  env.load("app.js");

  return { env, nav, toggle, toggleLabel, toggleUse, linkHome, linkSettings, pageHome, pageSettings, shown };
}

test("route bascule les sections, aria-current et replie le menu sur les pages avec sous-menu", () => {
  const { env, nav, toggleLabel, toggleUse, linkHome, linkSettings, pageHome, pageSettings, shown } = setupNav({
    hash: "#accueil",
  });

  assert.deepEqual(shown, ["accueil"]);
  assert.equal(pageHome.hidden, false);
  assert.equal(pageSettings.hidden, true);
  assert.equal(linkHome.getAttribute("aria-current"), "page");
  assert.equal(linkSettings.getAttribute("aria-current"), null);
  assert.equal(nav.classList.contains("compact"), false);
  assert.equal(toggleLabel.textContent, "Réduire le menu");
  assert.equal(toggleUse.getAttribute("href"), "#i-panel-left-close");

  // Navigation vers #reglages/nettoyage : sous-menu présent → menu principal replié d'office.
  env.location.hash = "#reglages/nettoyage";
  env.window.dispatchEvent("hashchange");

  assert.deepEqual(shown, ["accueil", "reglages"]);
  assert.equal(pageHome.hidden, true);
  assert.equal(pageSettings.hidden, false);
  assert.equal(linkHome.getAttribute("aria-current"), null);
  assert.equal(linkSettings.getAttribute("aria-current"), "page");
  assert.equal(nav.classList.contains("compact"), true);
  assert.equal(toggleLabel.textContent, "Déplier le menu");
  assert.equal(toggleUse.getAttribute("href"), "#i-panel-left-open");

  // Ancre inconnue : retour sur accueil.
  env.location.hash = "#inconnue";
  env.window.dispatchEvent("hashchange");
  assert.equal(pageHome.hidden, false);
  assert.equal(shown[shown.length - 1], "accueil");
});

test("MainNav mémorise le choix par type de page dans localStorage et anime après ouverture", () => {
  const { env, nav, toggle } = setupNav({ hash: "#accueil" });

  assert.equal(nav.classList.contains("animated"), false);
  env.flushRaf();
  assert.equal(nav.classList.contains("animated"), true);

  // Réduire sur une page normale ne modifie que la préférence `page`.
  toggle.click();
  assert.equal(nav.classList.contains("compact"), true);
  assert.deepEqual(JSON.parse(env.localStorage.getItem("vtt-main-nav-compact")), { sub: true, page: true });

  // Bascule vers Réglages (sub) puis dépliage : garde `page: true`, met `sub: false`.
  env.location.hash = "#reglages";
  env.window.dispatchEvent("hashchange");
  toggle.click();
  assert.equal(nav.classList.contains("compact"), false);
  assert.deepEqual(JSON.parse(env.localStorage.getItem("vtt-main-nav-compact")), { sub: false, page: true });

  // Préférence illisible dans localStorage : valeurs par défaut sans erreur.
  const fresh = setupNav({ hash: "#reglages", storage: { "vtt-main-nav-compact": "{invalide" } });
  assert.equal(fresh.nav.classList.contains("compact"), true);
});
