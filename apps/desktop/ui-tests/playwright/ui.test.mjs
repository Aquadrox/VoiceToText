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

      // Section Reconnaissance locale : prétraitement audio micro activé par défaut.
      await page.locator('.settings-nav a[data-panel-link="reconnaissance"]').click();
      await page.locator("#audio-enhance").waitFor({ state: "visible" });
      assert.equal(await page.locator("#audio-enhance").isChecked(), true);

      // Section Réunions : arrêt automatique à la fin d'un appel et délai associé.
      await page.locator('.settings-nav a[data-panel-link="reunions"]').click();
      await page.locator("#meeting-auto-stop").waitFor({ state: "visible" });
      assert.equal(await page.locator("#meeting-auto-stop").isChecked(), true);
      assert.equal(await page.locator("#meeting-auto-stop-delay").isDisabled(), false);
      assert.equal(await page.locator("#meeting-auto-stop-delay").inputValue(), "15");
      assert.equal(await page.locator("#meeting-auto-stop-silence").inputValue(), "0");
      await page.locator("#meeting-auto-stop").click();
      assert.equal(await page.locator("#meeting-auto-stop-delay").isDisabled(), true);

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
      const badges = await page.locator("#search-results .search-badge").allTextContents();
      assert.deepEqual(badges, ["Exact + sens", "Sens proche"]);
      await page.locator("#search-results button").first().click();
      await page.waitForTimeout(100);

      assert.equal(await page.locator("#meeting-panel-report").isVisible(), true);
      const hitText = await frame.locator("mark.hit").first().textContent();
      assert.equal(hitText?.toLowerCase(), "budget");

      // Question au LLM avec RAG sémantique.
      await page.locator("#ask-question").fill("Quelles dépenses ont été validées ?");
      await page.locator("#ask-send").click();
      await page.waitForSelector("#ask-answer:not([hidden])");
      assert.match(await page.locator("#ask-answer").textContent(), /recherche sémantique/);

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

      await page.evaluate(() => {
        window.__VTT_EMIT__("status", { state: "idle" });
        window.__VTT_EMIT__("meeting", {
          recording: true,
          conference: false,
          elapsed_ms: 90_000,
          consent: false,
          stopping_in_sec: 12,
        });
      });
      await page.waitForTimeout(60);
      assert.match(await page.locator("#label").textContent(), /fin 12 s/);

      await page.evaluate(() => {
        window.__VTT_EMIT__("meeting", { recording: false, elapsed_ms: 0, consent: false });
        window.__VTT_EMIT__("meeting-auto-stopped", "Fin de l'appel Teams : réunion arrêtée");
      });
      await page.waitForTimeout(60);
      assert.match(await page.locator("#label").textContent(), /Fin de l'appel Teams/);
      assert.deepEqual(errors, []);
    } finally {
      await context.close();
    }
  });

  it("floute une zone sensible dans review.html avant envoi au LLM vision et collage", async () => {
    const { context, page, errors } = await openUiPage(browser, server.baseUrl, {
      pagePath: "/review.html",
      viewport: { width: 900, height: 680 },
    });
    try {
      await page.waitForSelector("#review-list canvas");
      assert.equal(await page.locator("#review-vision-banner").isVisible(), true);
      assert.match(await page.locator("#review-apply").textContent(), /envoyer au LLM/);

      const canvas = page.locator("#review-list canvas").first();
      const box = await canvas.boundingBox();
      assert.ok(box && box.width > 50 && box.height > 30);

      // Trace une première zone (style par défaut « flou »).
      await page.mouse.move(box.x + 20, box.y + 20);
      await page.mouse.down();
      await page.mouse.move(box.x + 120, box.y + 80);
      await page.mouse.up();
      assert.equal(await page.locator("#review-count").textContent(), "1 zone");

      // Passe en style « plein », trace une seconde zone puis annule la dernière.
      await page.locator('[data-style="plein"]').click();
      await page.mouse.move(box.x + 140, box.y + 30);
      await page.mouse.down();
      await page.mouse.move(box.x + 220, box.y + 90);
      await page.mouse.up();
      assert.equal(await page.locator("#review-count").textContent(), "2 zones");

      await page.locator("#review-undo").click();
      assert.equal(await page.locator("#review-count").textContent(), "1 zone");

      await page.locator("#review-apply").click();
      await page.waitForTimeout(60);

      const calls = await page.evaluate(() => window.__VTT_CALLS__);
      const applyCall = calls.find((c) => c.cmd === "annotate_review_apply");
      assert.ok(applyCall, "annotate_review_apply doit être appelé");
      assert.equal(applyCall.args.redactions.length, 1);
      assert.equal(applyCall.args.redactions[0].image_index, 0);
      assert.equal(applyCall.args.redactions[0].style, "flou");
      assert.ok(applyCall.args.redactions[0].width > 10);
      assert.ok(applyCall.args.redactions[0].height > 10);

      assert.deepEqual(errors, []);
    } finally {
      await context.close();
    }
  });
});
