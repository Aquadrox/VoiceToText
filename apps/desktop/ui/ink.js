// Encre de la capture commentée : pendant que le raccourci est tenu, la
// souris dessine (sans clic) ; un clic-glisser gauche trace une zone sensible
// à flouter / masquer (appliquée à toutes les images avant collage ou envoi
// au LLM vision). Un clic droit retire la dernière zone masquée.
const canvas = document.getElementById("ink");
const context = canvas.getContext("2d");
const INK = "rgb(230, 30, 60)";
const HOLD_MS = 650;
const FADE_MS = 650;
/** Traits affichés : { points, endedAt } (endedAt null : en cours). */
let strokes = [];
let current = null;
let drawing = false;
/** Zones floutées validées pendant cette capture : [[x, y, w, h], …]. */
let redactions = [];
/** Zone floutée en cours de tracé : { start: [x, y], end: [x, y] } ou null. */
let activeRedact = null;
/** Tracé de la zone à capturer : { start: [x, y], end: [x, y] } ou null. */
let zone = null;
let choosingZone = false;

function resize() {
  const ratio = window.devicePixelRatio || 1;
  canvas.width = Math.round(window.innerWidth * ratio);
  canvas.height = Math.round(window.innerHeight * ratio);
  canvas.style.width = `${window.innerWidth}px`;
  canvas.style.height = `${window.innerHeight}px`;
  context.setTransform(ratio, 0, 0, ratio, 0, 0);
}
window.addEventListener("resize", resize);
resize();

function finishStroke() {
  if (!current) return;
  current.endedAt = performance.now();
  window.__TAURI__.core.invoke("annotate_stroke", { points: current.points }).catch(() => {});
  current = null;
}

function rectFromDrag(drag) {
  const [x0, y0] = drag.start;
  const [x1, y1] = drag.end;
  return [Math.min(x0, x1), Math.min(y0, y1), Math.abs(x1 - x0), Math.abs(y1 - y0)];
}

function finishRedaction() {
  if (!activeRedact) return;
  const rect = rectFromDrag(activeRedact);
  activeRedact = null;
  if (rect[2] >= 6 && rect[3] >= 6) {
    redactions.push(rect);
    window.__TAURI__.core.invoke("annotate_redact", { rect }).catch(() => {});
  }
}

// Zone : glisser pour tracer le rectangle ; Échap : tout l'écran.
function endZone(rect) {
  choosingZone = false;
  zone = null;
  document.body.classList.remove("drawing");
  window.__TAURI__.core.invoke("annotate_zone", { zone: rect }).catch(() => {});
}

function startZone() {
  choosingZone = true;
  zone = null;
  document.body.classList.add("drawing");
}
window.__TAURI__.event.listen("ink-zone", startZone);
// Page chargée après l'événement : l'état le dit.
window.__TAURI__.core
  .invoke("annotate_state")
  .then((state) => state.choosing_zone && !choosingZone && startZone())
  .catch(() => {});

document.addEventListener("mousedown", (event) => {
  if (choosingZone) {
    zone = { start: [event.clientX, event.clientY], end: [event.clientX, event.clientY] };
    return;
  }
  if (drawing && event.button === 0) {
    // Bascule proprement du trait libre vers le rectangle de floutage.
    if (current) {
      if (current.points.length < 5) {
        strokes = strokes.filter((s) => s !== current);
        current = null;
      } else {
        finishStroke();
      }
    }
    activeRedact = { start: [event.clientX, event.clientY], end: [event.clientX, event.clientY] };
  }
});

document.addEventListener("mouseup", () => {
  if (choosingZone && zone) {
    endZone(rectFromDrag(zone));
    return;
  }
  if (activeRedact) finishRedaction();
});

document.addEventListener("contextmenu", (event) => {
  if (!drawing) return;
  event.preventDefault();
  if (redactions.length > 0) {
    redactions.pop();
    window.__TAURI__.core.invoke("annotate_undo_redact").catch(() => {});
  }
});

document.addEventListener("keydown", (event) => {
  if (choosingZone && event.key === "Escape") endZone(null);
});

window.__TAURI__.event.listen("ink-draw", (event) => {
  drawing = Boolean(event.payload);
  document.body.classList.toggle("drawing", drawing);
  if (!drawing) {
    finishRedaction();
    finishStroke();
  }
});

document.addEventListener("mousemove", (event) => {
  if (choosingZone && zone) zone.end = [event.clientX, event.clientY];
  if (activeRedact) {
    activeRedact.end = [event.clientX, event.clientY];
    return;
  }
  if (!drawing) return;
  if (!current) {
    current = { points: [], endedAt: null };
    strokes.push(current);
  }
  current.points.push([event.clientX, event.clientY]);
});

function drawRedactBox([x, y, w, h], preview) {
  context.save();
  context.fillStyle = preview ? "rgba(24, 29, 39, 0.42)" : "rgba(24, 29, 39, 0.62)";
  context.fillRect(x, y, w, h);
  context.strokeStyle = preview ? INK : "rgba(255, 255, 255, 0.85)";
  context.lineWidth = 2;
  context.setLineDash([6, 4]);
  context.strokeRect(x, y, w, h);
  if (!preview && w >= 54 && h >= 22) {
    context.setLineDash([]);
    context.fillStyle = "rgba(255, 255, 255, 0.92)";
    context.font = "600 12px 'Segoe UI', system-ui, sans-serif";
    context.textAlign = "center";
    context.textBaseline = "middle";
    context.fillText("Flouté", x + w / 2, y + h / 2);
  }
  context.restore();
}

function render(now) {
  context.clearRect(0, 0, window.innerWidth, window.innerHeight);
  for (const rect of redactions) drawRedactBox(rect, false);
  if (activeRedact) drawRedactBox(rectFromDrag(activeRedact), true);

  strokes = strokes.filter((s) => s.endedAt === null || now - s.endedAt < HOLD_MS + FADE_MS);
  context.lineCap = "round";
  context.lineJoin = "round";
  for (const stroke of strokes) {
    const age = stroke.endedAt === null ? 0 : now - stroke.endedAt;
    context.globalAlpha = age < HOLD_MS ? 1 : Math.max(0, 1 - (age - HOLD_MS) / FADE_MS);
    context.beginPath();
    stroke.points.forEach(([x, y], i) => (i ? context.lineTo(x, y) : context.moveTo(x, y)));
    context.strokeStyle = "white";
    context.lineWidth = 7;
    context.stroke();
    context.strokeStyle = INK;
    context.lineWidth = 4;
    context.stroke();
  }
  context.globalAlpha = 1;
  if (drawing && !choosingZone) {
    const label = "Bouger : dessiner · Clic gauche glissé : flouter · Clic droit : annuler";
    context.save();
    context.font = "600 13px 'Segoe UI', system-ui, sans-serif";
    const width = context.measureText(label).width + 28;
    const x = (window.innerWidth - width) / 2;
    context.fillStyle = "rgba(24, 29, 39, 0.85)";
    context.beginPath();
    context.roundRect(x, 16, width, 30, 15);
    context.fill();
    context.fillStyle = "white";
    context.textAlign = "center";
    context.textBaseline = "middle";
    context.fillText(label, window.innerWidth / 2, 31);
    context.restore();
  }
  if (choosingZone) {
    // Écran voilé, zone tracée en clair.
    context.fillStyle = "rgba(0, 0, 0, 0.35)";
    context.fillRect(0, 0, window.innerWidth, window.innerHeight);
    if (zone) {
      const [x, y, w, h] = rectFromDrag(zone);
      context.clearRect(x, y, w, h);
      context.strokeStyle = INK;
      context.lineWidth = 2;
      context.strokeRect(x, y, w, h);
    } else {
      context.fillStyle = "white";
      context.font = "600 20px 'Segoe UI', system-ui, sans-serif";
      context.textAlign = "center";
      context.fillText("Tracez la zone à capturer (Échap : tout l'écran)", window.innerWidth / 2, 48);
    }
  }
  requestAnimationFrame(render);
}
requestAnimationFrame(render);
