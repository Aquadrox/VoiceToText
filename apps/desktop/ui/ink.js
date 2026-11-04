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

window.__TAURI__.event.listen("ink-draw", (event) => {
  drawing = Boolean(event.payload);
  document.body.classList.toggle("drawing", drawing);
  if (!drawing) finishStroke();
});

document.addEventListener("mousemove", (event) => {
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
  requestAnimationFrame(render);
}
requestAnimationFrame(render);
