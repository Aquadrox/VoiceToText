// Génère les icônes de l'application à partir de icon.svg et icon-small.svg.
// Usage (Playwright et Chromium requis) : node apps/desktop/icons/render-icons.js
// Chaque taille est rendue directement depuis le SVG (pas de réduction d'une
// grande image) ; l'.ico contient des PNG, format lu par Windows depuis Vista.
const fs = require("fs");
const path = require("path");
const { chromium } = require("playwright");

const dir = __dirname;
const big = fs.readFileSync(path.join(dir, "icon.svg"), "utf8");
const small = fs.readFileSync(path.join(dir, "icon-small.svg"), "utf8");
const svgFor = (size) => (size <= 32 ? small : big);

async function render(page, size) {
  const src = `data:image/svg+xml;base64,${Buffer.from(svgFor(size)).toString("base64")}`;
  await page.setViewportSize({ width: size, height: size });
  await page.setContent(
    `<body style="margin:0;background:transparent"><img src="${src}" width="${size}" height="${size}" style="display:block"></body>`,
  );
  await page.waitForFunction(() => document.images[0].complete);
  return page.screenshot({ omitBackground: true, clip: { x: 0, y: 0, width: size, height: size } });
}

/** .ico : en-tête, une entrée par taille, puis les PNG à la suite. */
function ico(images) {
  const header = Buffer.alloc(6);
  header.writeUInt16LE(0, 0);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(images.length, 4);
  let offset = 6 + 16 * images.length;
  const entries = images.map(({ size, png }) => {
    const entry = Buffer.alloc(16);
    entry.writeUInt8(size >= 256 ? 0 : size, 0);
    entry.writeUInt8(size >= 256 ? 0 : size, 1);
    entry.writeUInt16LE(1, 4);
    entry.writeUInt16LE(32, 6);
    entry.writeUInt32LE(png.length, 8);
    entry.writeUInt32LE(offset, 12);
    offset += png.length;
    return entry;
  });
  return Buffer.concat([header, ...entries, ...images.map((i) => i.png)]);
}

(async () => {
  const browser = await chromium.launch();
  const page = await browser.newPage({ deviceScaleFactor: 1 });
  const files = { "32x32.png": 32, "128x128.png": 128, "128x128@2x.png": 256, "icon.png": 1024 };
  for (const [name, size] of Object.entries(files)) {
    fs.writeFileSync(path.join(dir, name), await render(page, size));
  }
  const sizes = [16, 20, 24, 32, 48, 64, 128, 256];
  const images = [];
  for (const size of sizes) images.push({ size, png: await render(page, size) });
  fs.writeFileSync(path.join(dir, "icon.ico"), ico(images));
  await browser.close();
  console.log("Icônes générées :", Object.keys(files).join(", "), "et icon.ico", sizes.join("/"));
})();
