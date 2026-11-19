import test from "node:test";
import assert from "node:assert/strict";
import { createEnvironment } from "./dom.mjs";

test("formatDuration formate secondes, minutes et heures avec zéro initial", () => {
  const env = createEnvironment();
  env.load("common.js");
  const formatDuration = env.eval("formatDuration");

  assert.equal(formatDuration(0), "0 s");
  assert.equal(formatDuration(45_000), "45 s");
  assert.equal(formatDuration(60_000), "1 min");
  assert.equal(formatDuration(12 * 60_000), "12 min");
  assert.equal(formatDuration(185 * 60_000), "3 h 05");
});

test("formatCost gère montants minuscules, normaux et devise invalide", () => {
  const env = createEnvironment();
  env.load("common.js");
  const formatCost = env.eval("formatCost");

  const tiny = formatCost(0.0001234, "USD");
  assert.match(tiny, /0,00012/);
  const regular = formatCost(1.5, "EUR");
  assert.match(regular, /1,50/);
  const zero = formatCost(0, "USD");
  assert.match(zero, /0,00/);

  // Devise invalide (pas un code ISO à 3 lettres) : repli sans exception.
  assert.equal(formatCost(0.0001234, "INVALID"), "0.00012 INVALID");
  assert.equal(formatCost(2.5, "INVALID"), "2.50 INVALID");
});

test("currentPage, currentSection et isShown décodent l'ancre et la visibilité", () => {
  const env = createEnvironment({ hash: "" });
  env.load("common.js");
  const currentPage = env.eval("currentPage");
  const currentSection = env.eval("currentSection");
  const isShown = env.eval("isShown");

  assert.equal(currentPage(), "accueil");
  assert.equal(currentSection(), "");
  assert.equal(isShown("accueil"), true);

  env.location.hash = "#reglages/nettoyage";
  assert.equal(currentPage(), "reglages");
  assert.equal(currentSection(), "nettoyage");
  assert.equal(isShown("reglages"), true);
  assert.equal(isShown("accueil"), false);

  env.document.hidden = true;
  assert.equal(isShown("reglages"), false);
});

test("renderReleases crée les blocs, mémorise l'état, cache si vide et échappe le texte", () => {
  const env = createEnvironment();
  env.load("common.js");
  const renderReleases = env.eval("renderReleases");

  const container = env.el("div");
  const releases = [
    { version: "0.35.1", date: "18 novembre 2026", items: ["<script>alert(1)</script>", "Bouton corrigé"] },
    { version: "0.35.0", date: "", items: ["Rapport complet"] },
  ];

  renderReleases(container, releases, 1);
  assert.equal(container.hidden, false);
  assert.equal(container.children.length, 2);
  assert.equal(container.children[0].tagName, "DETAILS");
  assert.equal(container.children[0].open, true);
  assert.equal(container.children[1].open, false);
  assert.equal(container.children[0].querySelector("summary").textContent, "Version 0.35.1 · 18 novembre 2026");
  assert.equal(container.children[1].querySelector("summary").textContent, "Version 0.35.0");
  // Le texte contenant du HTML reste du texte pur (textContent).
  assert.equal(container.children[0].querySelectorAll("li")[0].textContent, "<script>alert(1)</script>");

  // Appel identique : ne reconstruit pas le DOM (l'état ouvert choisi par l'utilisateur est conservé).
  const firstDetails = container.children[0];
  firstDetails.open = false;
  renderReleases(container, releases, 1);
  assert.equal(container.children[0], firstDetails);
  assert.equal(container.children[0].open, false);

  // Liste vide : conteneur masqué.
  renderReleases(container, []);
  assert.equal(container.hidden, true);
  assert.equal(container.children.length, 0);
});
