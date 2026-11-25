import { describe, it, before, after } from "node:test";
import assert from "node:assert/strict";
import { launchBrowser, openUiPage, startUiServer } from "./harness.mjs";

describe("Interface PC VoiceToText (Playwright)", () => {
  let browser;
  let server;

  before(async () => {
    server = await startUiServer();
    browser = await launchBrowser();
  });

  after(async () => {
    await browser?.close();
    await server?.close();
  });

  for (const colorScheme of ["light", "dark"]) {
    it(`parcourt toutes les pages sans erreur ni débordement horizontal (${colorScheme}, 980×720 et 720×480)`, async () => {
      const { context, page, errors } = await openUiPage(browser, server.baseUrl, {
        colorScheme,
        viewport: { width: 980, height: 720 },
      });
      try {
        const pages = [
          "#accueil",
          "#reunions",
          "#historique",
          "#dictionnaire",
          "#snippets",
          "#styles",
          "#prompts",
          "#reglages/raccourcis",
        ];

        for (const hash of pages) {
          await page.evaluate((h) => {
            window.location.hash = h;
          }, hash);
          await page.waitForTimeout(80);
          const overflow = await page.evaluate(() => ({
            scrollWidth: document.documentElement.scrollWidth,
            clientWidth: document.documentElement.clientWidth,
          }));
          assert.ok(
            overflow.scrollWidth <= overflow.clientWidth + 1,
            `Débordement horizontal sur ${hash} (${colorScheme}, 980×720): ${overflow.scrollWidth} > ${overflow.clientWidth}`,
          );
        }

        // Menu principal replié automatiquement sur Réglages (v0.27), déplié sur Accueil.
        assert.equal(await page.locator("#main-nav").evaluate((el) => el.classList.contains("compact")), true);
        await page.evaluate(() => {
          window.location.hash = "#accueil";
        });
        await page.waitForTimeout(80);
        assert.equal(await page.locator("#main-nav").evaluate((el) => el.classList.contains("compact")), false);

        // Fenêtre minimale 720×480 : aucun débordement non plus.
        await page.setViewportSize({ width: 720, height: 480 });
        for (const hash of ["#accueil", "#reunions", "#reglages/reunions"]) {
          await page.evaluate((h) => {
            window.location.hash = h;
          }, hash);
          await page.waitForTimeout(80);
          const overflow = await page.evaluate(() => ({
            scrollWidth: document.documentElement.scrollWidth,
            clientWidth: document.documentElement.clientWidth,
          }));
          assert.ok(
            overflow.scrollWidth <= overflow.clientWidth + 1,
            `Débordement horizontal sur ${hash} (${colorScheme}, 720×480): ${overflow.scrollWidth} > ${overflow.clientWidth}`,
          );
        }

        assert.deepEqual(errors, []);
      } finally {
        await context.close();
      }
    });
  }

  it("vérifie les sections Réglages, le suivi des modifications et le contraste des listes en mode sombre", async () => {
    const { context, page, errors } = await openUiPage(browser, server.baseUrl, {
      colorScheme: "dark",
      hash: "#reglages/raccourcis",
    });
    try {
      const sections = [
        "raccourcis",
        "reconnaissance",
        "dictee",
        "nettoyage",
        "cles",
        "reunions",
        "capture",
        "synchronisation",
        "application",
      ];
      for (const section of sections) {
        await page.locator(`.settings-nav a[data-panel-link="${section}"]`).click();
        await page.waitForTimeout(50);
        const visible = await page.locator(`.panel[data-panel="${section}"]`).isVisible();
        assert.equal(visible, true, `Section ${section} invisible`);
      }

      // Non-régression v0.32 : les <select> en mode sombre doivent rester lisibles.
      await page.locator('.settings-nav a[data-panel-link="raccourcis"]').click();
      const colors = await page.locator('.panel[data-panel="raccourcis"] select').first().evaluate((el) => {
        const s = getComputedStyle(el);
        return { color: s.color, bg: s.backgroundColor };
      });
      assert.notEqual(colors.color, colors.bg, "Liste déroulante illisible en mode sombre");

      // Modification puis Annuler remet la section propre.
      await page.locator('.settings-nav a[data-panel-link="dictee"]').click();
      const checkbox = page.locator("#dict-sounds");
      await checkbox.waitFor({ state: "visible" });
      const before = await checkbox.isChecked();
      await checkbox.click();
      await page.waitForTimeout(60);
      const dirtyText = await page.locator('.panel[data-panel="dictee"] .apply-state').textContent();
      assert.match(dirtyText || "", /modification/);
      await page.locator('.panel[data-panel="dictee"] button.revert').click();
      assert.equal(await checkbox.isChecked(), before);

      assert.deepEqual(errors, []);
    } finally {
      await context.close();
    }
  });

  it("affiche la page Réunions, l'onglet Rapport complet (iframe + window.VTT) et la recherche globale", async () => {
    const { context, page, errors } = await openUiPage(browser, server.baseUrl, {
      colorScheme: "light",
      hash: "#reunions",
    });
    try {
      await page.waitForSelector("#meeting-detail:not([hidden])");
      assert.match(await page.locator("#meeting-list").textContent(), /Comité produit/);
      assert.match(await page.locator("#meeting-speakers").textContent(), /94 %/);

      // Bascule vers l'onglet Rapport complet : l'iframe charge et coche une action.
      await page.locator("#meeting-tab-btn-report").click();
      await page.waitForSelector("#meeting-report-frame:not([hidden])");

      const frame = page.frameLocator("#meeting-report-frame");
      await frame.locator("#synthese").waitFor({ state: "visible" });
      await frame.locator(".action input[type=checkbox]").check();
      await page.waitForTimeout(80);

      const calls = await page.evaluate(() => window.__VTT_CALLS__);
      assert.ok(
        calls.some(
          (c) =>
            c.cmd === "set_meeting_report_action_done" &&
            c.args?.id === "m-1" &&
            c.args?.index === 0 &&
            c.args?.done === true,
        ),
        "Cocher une action dans l'iframe doit appeler set_meeting_report_action_done",
      );

      // Retour sur Transcription puis clic sur un résultat de recherche globale issu du rapport.
      await page.locator("#meeting-tab-btn-transcript").click();
      await page.locator("#meeting-search").fill("budget");
      await page.waitForSelector("#search-results:not([hidden]) button");
      await page.locator("#search-results button").first().click();
      await page.waitForTimeout(100);

      assert.equal(await page.locator("#meeting-panel-report").isVisible(), true);
      const hitText = await frame.locator("mark.hit").first().textContent();
      assert.equal(hitText?.toLowerCase(), "budget");

      assert.deepEqual(errors, []);
    } finally {
      await context.close();
    }
  });

  it("affiche les 5 types de prompts dont les consignes de style du rapport et enregistre", async () => {
    const { context, page, errors } = await openUiPage(browser, server.baseUrl, {
      colorScheme: "light",
      hash: "#prompts",
    });
    try {
      const titles = await page.locator("#prompt-kinds .kind-title").allTextContents();
      assert.equal(titles.length, 5);
      assert.ok(titles.some((t) => t.includes("rapport complet")));

      // Copie le prompt intégré du rapport et enregistre.
      const reportBlock = page.locator("#prompt-kinds > section").last();
      await reportBlock.locator("summary").click();
      await reportBlock.locator(".copy-default").click();
      await page.locator("#prompts-save").click();
      await page.waitForTimeout(80);

      const calls = await page.evaluate(() => window.__VTT_CALLS__);
      const saveCall = calls.find((c) => c.cmd === "save_prompts");
      assert.ok(saveCall, "save_prompts doit être appelé");
      assert.equal(saveCall.args.prompts.length, 1);
      assert.equal(saveCall.args.prompts[0].kind, "rapport");

      assert.deepEqual(errors, []);
    } finally {
      await context.close();
    }
  });

  it("affiche la pastille flottante (index.html) dans ses états clés sans erreur", async () => {
    const { context, page, errors } = await openUiPage(browser, server.baseUrl, {
      pagePath: "/index.html",
      viewport: { width: 320, height: 80 },
    });
    try {
      await page.evaluate(() => {
        window.__VTT_EMIT__("status", { state: "handsfree" });
        window.__VTT_EMIT__("level", 0.65);
        window.__VTT_EMIT__("live-preview", "Bonjour tout le monde");
      });
      await page.waitForTimeout(60);
      assert.match(await page.locator("#preview").textContent(), /Bonjour tout le monde/);

      await page.evaluate(() => {
        window.__VTT_EMIT__("status", { state: "processing" });
      });
      await page.waitForTimeout(60);
      assert.match(await page.locator("#label").textContent(), /Transcription/);
      assert.deepEqual(errors, []);
    } finally {
      await context.close();
    }
  });
});
