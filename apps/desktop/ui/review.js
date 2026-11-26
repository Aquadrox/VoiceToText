// Fenêtre rapide de floutage / masquage avant collage ou envoi au LLM vision.
// Chaque rectangle est tracé sur le canvas de l'image, prévisualisé aussitôt,
// puis envoyé en pixels natifs de l'image au moteur Rust (`annotate_review_apply`).

const invoke = window.__TAURI__.core.invoke;
const $ = (id) => document.getElementById(id);

const REDACT_SOLID = "#181d27";
const SIGNAL = "#e61e3c";

let currentStyle = "flou";
/** Entrées chargées : { canvas, ctx, img, width, height, drag } */
let cards = [];
/** Historique global des zones masquées : { image_index, x, y, width, height, style } */
let redactions = [];
let submitting = false;

function updateToolbar() {
  for (const btn of document.querySelectorAll("[data-style]")) {
    btn.setAttribute("aria-pressed", String(btn.dataset.style === currentStyle));
  }
  const n = redactions.length;
  $("review-count").textContent = n <= 1 ? `${n} zone` : `${n} zones`;
  $("review-undo").disabled = n === 0 || submitting;
  $("review-clear").disabled = n === 0 || submitting;
  $("review-apply").disabled = submitting || cards.length === 0;
}

for (const btn of document.querySelectorAll("[data-style]")) {
  btn.addEventListener("click", () => {
    currentStyle = btn.dataset.style;
    updateToolbar();
  });
}

/** Prévisualisation fidèle d'un rectangle masqué sur le canvas de l'image. */
function drawRedactionPreview(card, r, isDraft) {
  const { ctx, img } = card;
  const x = Math.max(0, Math.min(card.width, Math.round(r.x)));
  const y = Math.max(0, Math.min(card.height, Math.round(r.y)));
  const w = Math.max(0, Math.min(card.width - x, Math.round(r.width)));
  const h = Math.max(0, Math.min(card.height - y, Math.round(r.height)));
  if (w < 2 || h < 2) return;

  ctx.save();
  if (r.style === "plein") {
    ctx.fillStyle = REDACT_SOLID;
    ctx.fillRect(x, y, w, h);
  } else {
    // Mosaïque / flou rapide côté canvas pour aperçu immédiat.
    const block = Math.max(8, Math.min(20, Math.floor(Math.min(w, h) / 4)));
    const sw = Math.max(1, Math.ceil(w / block));
    const sh = Math.max(1, Math.ceil(h / block));
    const off = document.createElement("canvas");
    off.width = sw;
    off.height = sh;
    const octx = off.getContext("2d");
    octx.drawImage(img, x, y, w, h, 0, 0, sw, sh);
    ctx.imageSmoothingEnabled = r.style === "flou";
    if (r.style === "flou") ctx.filter = "blur(6px)";
    ctx.beginPath();
    ctx.rect(x, y, w, h);
    ctx.clip();
    ctx.drawImage(off, 0, 0, sw, sh, x, y, w, h);
    ctx.filter = "none";
  }
  ctx.strokeStyle = isDraft ? SIGNAL : "rgba(255, 255, 255, 0.85)";
  ctx.lineWidth = Math.max(2, Math.round(card.width / 450));
  ctx.setLineDash(isDraft ? [8, 5] : [4, 4]);
  ctx.strokeRect(x + 1, y + 1, Math.max(1, w - 2), Math.max(1, h - 2));
  ctx.restore();
}

function redrawCard(index) {
  const card = cards[index];
  if (!card) return;
  card.ctx.clearRect(0, 0, card.width, card.height);
  card.ctx.drawImage(card.img, 0, 0, card.width, card.height);
  for (const r of redactions) {
    if (r.image_index === index) drawRedactionPreview(card, r, false);
  }
  if (card.drag) {
    const { x0, y0, x1, y1 } = card.drag;
    drawRedactionPreview(
      card,
      {
        x: Math.min(x0, x1),
        y: Math.min(y0, y1),
        width: Math.abs(x1 - x0),
        height: Math.abs(y1 - y0),
        style: currentStyle,
      },
      true,
    );
  }
}

function pointerToImageCoords(canvas, event) {
  const rect = canvas.getBoundingClientRect();
  const scaleX = rect.width > 0 ? canvas.width / rect.width : 1;
  const scaleY = rect.height > 0 ? canvas.height / rect.height : 1;
  return [
    Math.max(0, Math.min(canvas.width, (event.clientX - rect.left) * scaleX)),
    Math.max(0, Math.min(canvas.height, (event.clientY - rect.top) * scaleY)),
  ];
}

function buildCard(index, item) {
  const article = document.createElement("article");
  article.className = "card";

  const head = document.createElement("div");
  head.className = "card-head";
  const badge = document.createElement("span");
  badge.className = "badge-num";
  badge.textContent = `Image ${index + 1}`;
  const dims = document.createElement("span");
  dims.textContent = `${item.width} × ${item.height}`;
  head.append(badge, dims);

  const canvas = document.createElement("canvas");
  canvas.width = item.width;
  canvas.height = item.height;
  canvas.dataset.imageIndex = String(index);
  const ctx = canvas.getContext("2d");

  const img = new Image();
  const card = { canvas, ctx, img, width: item.width, height: item.height, drag: null };
  img.onload = () => redrawCard(index);
  img.src = item.data_url;

  canvas.addEventListener("pointerdown", (event) => {
    if (event.button !== 0 || submitting) return;
    canvas.setPointerCapture?.(event.pointerId);
    const [x, y] = pointerToImageCoords(canvas, event);
    card.drag = { x0: x, y0: y, x1: x, y1: y };
    redrawCard(index);
  });

  canvas.addEventListener("pointermove", (event) => {
    if (!card.drag) return;
    const [x, y] = pointerToImageCoords(canvas, event);
    card.drag.x1 = x;
    card.drag.y1 = y;
    redrawCard(index);
  });

  const finishDrag = () => {
    if (!card.drag) return;
    const { x0, y0, x1, y1 } = card.drag;
    card.drag = null;
    const x = Math.round(Math.min(x0, x1));
    const y = Math.round(Math.min(y0, y1));
    const width = Math.round(Math.abs(x1 - x0));
    const height = Math.round(Math.abs(y1 - y0));
    if (width >= 4 && height >= 4) {
      redactions.push({ image_index: index, x, y, width, height, style: currentStyle });
    }
    redrawCard(index);
    updateToolbar();
  };

  canvas.addEventListener("pointerup", finishDrag);
  canvas.addEventListener("pointercancel", () => {
    card.drag = null;
    redrawCard(index);
  });

  article.append(head, canvas);
  return { article, card };
}

function undoLast() {
  if (redactions.length === 0 || submitting) return;
  const removed = redactions.pop();
  redrawCard(removed.image_index);
  updateToolbar();
}

function clearAll() {
  if (redactions.length === 0 || submitting) return;
  redactions = [];
  for (let i = 0; i < cards.length; i++) redrawCard(i);
  updateToolbar();
}

async function applyReview() {
  if (submitting || cards.length === 0) return;
  submitting = true;
  updateToolbar();
  $("review-error").hidden = true;
  try {
    await invoke("annotate_review_apply", { redactions });
  } catch (e) {
    submitting = false;
    $("review-error").textContent = String(e);
    $("review-error").hidden = false;
    updateToolbar();
  }
}

async function cancelReview() {
  if (submitting) return;
  submitting = true;
  await invoke("annotate_review_cancel").catch(() => {});
}

$("review-undo").addEventListener("click", undoLast);
$("review-clear").addEventListener("click", clearAll);
$("review-apply").addEventListener("click", applyReview);
$("review-cancel").addEventListener("click", cancelReview);

document.addEventListener("keydown", (event) => {
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "z") {
    event.preventDefault();
    undoLast();
  } else if (event.key === "Escape") {
    event.preventDefault();
    cancelReview();
  } else if (event.key === "Enter") {
    event.preventDefault();
    applyReview();
  }
});

async function loadReview() {
  submitting = false;
  redactions = [];
  cards = [];
  $("review-error").hidden = true;
  try {
    const view = await invoke("annotate_review_view");
    currentStyle = ["flou", "pixel", "plein"].includes(view.style) ? view.style : "flou";
    $("review-vision-banner").hidden = !view.vision_enabled;
    $("review-apply").textContent = view.vision_enabled
      ? "Valider, envoyer au LLM et coller"
      : "Valider et coller";
    const list = $("review-list");
    const fragments = [];
    view.images.forEach((item, index) => {
      const { article, card } = buildCard(index, item);
      cards.push(card);
      fragments.push(article);
    });
    list.replaceChildren(...fragments);
  } catch (e) {
    $("review-error").textContent = String(e);
    $("review-error").hidden = false;
  }
  updateToolbar();
}

window.__TAURI__.event?.listen?.("review-reload", () => loadReview());
loadReview();
