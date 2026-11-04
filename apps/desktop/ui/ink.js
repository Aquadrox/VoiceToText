// Encre de la capture commentée : pendant que le raccourci est tenu, la
// souris dessine (sans clic). Les traits s'effacent à l'écran mais sont
// envoyés à l'application, qui les dessine sur les captures.
const canvas = document.getElementById("ink");
const context = canvas.getContext("2d");
const INK = "rgb(230, 30, 60)";
const HOLD_MS = 650;
const FADE_MS = 650;
/** Traits affichés : { points, endedAt } (endedAt null : en cours). */
let strokes = [];
let current = null;
let drawing = false;
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
  if (choosingZone) zone = { start: [event.clientX, event.clientY], end: [event.clientX, event.clientY] };
});

document.addEventListener("mouseup", () => {
  if (!choosingZone || !zone) return;
  const [x0, y0] = zone.start;
  const [x1, y1] = zone.end;
  endZone([Math.min(x0, x1), Math.min(y0, y1), Math.abs(x1 - x0), Math.abs(y1 - y0)]);
});

document.addEventListener("keydown", (event) => {
  if (choosingZone && event.key === "Escape") endZone(null);
});

window.__TAURI__.event.listen("ink-draw", (event) => {
  drawing = Boolean(event.payload);
  document.body.classList.toggle("drawing", drawing);
  if (!drawing) finishStroke();
});

document.addEventListener("mousemove", (event) => {
  if (choosingZone && zone) zone.end = [event.clientX, event.clientY];
  if (!drawing) return;
  if (!current) {
    current = { points: [], endedAt: null };
    strokes.push(current);
  }
  current.points.push([event.clientX, event.clientY]);
});

function render(now) {
  context.clearRect(0, 0, window.innerWidth, window.innerHeight);
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
  if (choosingZone) {
    // Écran voilé, zone tracée en clair.
    context.fillStyle = "rgba(0, 0, 0, 0.35)";
    context.fillRect(0, 0, window.innerWidth, window.innerHeight);
    if (zone) {
      const [x0, y0] = zone.start;
      const [x1, y1] = zone.end;
      const [x, y, w, h] = [Math.min(x0, x1), Math.min(y0, y1), Math.abs(x1 - x0), Math.abs(y1 - y0)];
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
