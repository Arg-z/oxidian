/* Graph View — jendela terpisah Oxidian.
 * Fisika real-time ala Obsidian: node melayang halus, settle perlahan,
 * zoom/pan dengan easing, animasi spawn, hover yang lerp. */
"use strict";

const { invoke } = window.__TAURI__.core;
const { emit, listen } = window.__TAURI__.event;

const canvas = document.getElementById("gc");
const ctx = canvas.getContext("2d");

let nodes = [];
let edges = [];
let scale = 1;
let targetScale = 1;
let offset = { x: 0, y: 0 };
let targetOffset = { x: 0, y: 0 };
let hover = null;
let dragNode = null;
let panning = false;
let moved = 0;
let heat = 1;          // energi fisika — turun perlahan, tak pernah nol (node tetap "bernafas")
let clock = 0;
let ready = false;     // fade-in awal

function status(t) {
  document.getElementById("gstat").textContent = t;
}

async function loadGraph() {
  try {
    const data = await invoke("graph_data");
    // bangun objek BARU field-per-field — hasil invoke bisa ber-property
    // read-only di webview (strict mode), jadi jangan pernah pakai objeknya langsung
    nodes = data.nodes.map((n, i) => ({
      id: String(n.id),
      title: String(n.title),
      link_count: Number(n.link_count) || 0,
      x: 0, y: 0, vx: 0, vy: 0,
      _dr: 0, spawn: 0,
      phase: (i * 2.399963) % (Math.PI * 2), // golden-angle: sebaran fase stabil
    }));
    edges = data.edges.map((e) => ({
      source: String(e.source),
      target: String(e.target),
    }));
    document.getElementById("ghint").textContent = `${nodes.length} note • ${edges.length} link`;
    heat = 1;
    ready = 0;
    seedPositions();
    status("");
  } catch (e) {
    status(String(e).slice(0, 80));
  }
}

/// Posisi awal: lingkaran — fisika yang menyusun sisanya secara live.
function seedPositions() {
  const N = nodes.length;
  const W = canvas.clientWidth, H = canvas.clientHeight;
  const cx = W / 2, cy = H / 2;
  nodes.forEach((n, i) => {
    const a = (2 * Math.PI * i) / Math.max(1, N) - Math.PI / 2;
    n.x = cx + Math.cos(a) * Math.min(W, H) * 0.28;
    n.y = cy + Math.sin(a) * Math.min(W, H) * 0.28;
  });
}

/// Satu langkah fisika per frame — pendinginan perlahan, tak pernah mati total.
function tick() {
  const N = nodes.length;
  if (!N) return;
  const W = canvas.clientWidth, H = canvas.clientHeight;
  const cx = W / 2, cy = H / 2;
  const idx = new Map(nodes.map((n) => [n.id, n]));

  // repulsi (dengan cutoff jarak)
  for (let i = 0; i < N; i++) {
    for (let j = i + 1; j < N; j++) {
      const a = nodes[i], b = nodes[j];
      const dx = a.x - b.x, dy = a.y - b.y;
      const d2 = dx * dx + dy * dy || 1;
      if (d2 > 480 * 480) continue;
      const f = 9000 / d2, d = Math.sqrt(d2);
      const fx = (dx / d) * f, fy = (dy / d) * f;
      a.vx += fx; a.vy += fy;
      b.vx -= fx; b.vy -= fy;
    }
  }
  // pegas
  for (const e of edges) {
    const a = idx.get(e.source), b = idx.get(e.target);
    if (!a || !b) continue;
    const dx = b.x - a.x, dy = b.y - a.y;
    const d = Math.sqrt(dx * dx + dy * dy) || 1;
    const f = (d - 90) * 0.02;
    a.vx += (dx / d) * f; a.vy += (dy / d) * f;
    b.vx -= (dx / d) * f; b.vy -= (dy / d) * f;
  }
  // gravitasi pusat + integrasi (heat memudar ke 0.12 — node tetap melayang halus)
  for (const n of nodes) {
    n.vx += (cx - n.x) * 0.03;
    n.vy += (cy - n.y) * 0.03;
    const step = (v) => Math.max(-12, Math.min(12, v)) * heat;
    n.x += step(n.vx);
    n.y += step(n.vy);
    n.vx *= 0.6; n.vy *= 0.6;
    // napas: drift sinus kecil dengan fase unik per node
    n.x += Math.sin(clock * 0.7 + n.phase) * 0.05 * heat;
    n.y += Math.cos(clock * 0.6 + n.phase) * 0.05 * heat;
  }
  heat = Math.max(0.12, heat * 0.995);
}

function easeOutCubic(t) {
  return 1 - Math.pow(1 - t, 3);
}

function draw() {
  try {
    drawFrame();
  } catch (err) {
    status("DRAW ERR: " + err.message);
    return; // hentikan rAF — tampilkan error
  }
  requestAnimationFrame(draw);
}

function drawFrame() {
  const dpr = window.devicePixelRatio || 1;
  const W = canvas.clientWidth, H = canvas.clientHeight;
  if (canvas.width !== W * dpr) { canvas.width = W * dpr; canvas.height = H * dpr; }
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, W, H);

  // easing zoom & pan
  scale += (targetScale - scale) * 0.18;
  offset.x += (targetOffset.x - offset.x) * 0.18;
  offset.y += (targetOffset.y - offset.y) * 0.18;

  clock += 1 / 60;
  tick();

  ctx.save();
  ctx.translate(offset.x, offset.y);
  ctx.scale(scale, scale);
  ctx.globalAlpha = ready; // fade-in canvas saat pertama terbuka
  ready = Math.min(1, ready + 0.06);

  const idx = new Map(nodes.map((n) => [n.id, n]));
  // edges: alpha mengikuti spawn kedua ujung
  for (const e of edges) {
    const a = idx.get(e.source), b = idx.get(e.target);
    if (!a || !b) continue;
    ctx.globalAlpha = ready * easeOutCubic(Math.min(a.spawn, b.spawn)) * 0.7;
    ctx.strokeStyle = "rgba(150,140,200,0.6)";
    ctx.lineWidth = 1.2;
    ctx.beginPath();
    ctx.moveTo(a.x, a.y);
    ctx.lineTo(b.x, b.y);
    ctx.stroke();
  }
  for (const n of nodes) {
    n.spawn = Math.min(1, n.spawn + 0.04 + Math.random() * 0.015);
    const r = 6 + Math.min(10, n.link_count * 1.6);
    const hot = hover === n;
    // radius tampil di-lerp → hover membesar mulus
    n._dr += (r * easeOutCubic(n.spawn) * (hot ? 1.25 : 1) - n._dr) * 0.22;
    ctx.globalAlpha = ready * easeOutCubic(n.spawn);
    ctx.beginPath();
    ctx.arc(n.x, n.y, n._dr, 0, Math.PI * 2);
    ctx.fillStyle = hot ? "#c9b3ff" : "#7c5cff";
    ctx.fill();
    if (hot) { ctx.strokeStyle = "#fff"; ctx.lineWidth = 1.5; ctx.stroke(); }
    if (scale > 0.55 || hot) {
      ctx.fillStyle = hot ? "#fff" : "rgba(220,215,235,0.75)";
      ctx.font = "11px sans-serif";
      ctx.fillText(n.title, n.x + n._dr + 3, n.y + 4);
    }
  }
  ctx.restore();
}

function point(e) {
  const rect = canvas.getBoundingClientRect();
  return {
    x: (e.clientX - rect.left - offset.x) / scale,
    y: (e.clientY - rect.top - offset.y) / scale,
  };
}
function nodeAt(x, y) {
  for (const n of nodes) {
    const dx = n.x - x, dy = n.y - y;
    if (dx * dx + dy * dy <= (n._dr + 4) ** 2) return n;
  }
  return null;
}

canvas.addEventListener("mousemove", (e) => {
  const p = point(e);
  if (dragNode) {
    moved++;
    dragNode.x = p.x;
    dragNode.y = p.y;
    dragNode.vx = 0; dragNode.vy = 0;
    heat = Math.max(heat, 0.4); // lepasan drag → tetangga menyesuaikan secara halus
    return;
  }
  if (panning) {
    targetOffset.x += e.movementX;
    targetOffset.y += e.movementY;
    offset.x += e.movementX;
    offset.y += e.movementY;
    return;
  }
  hover = nodeAt(p.x, p.y);
  canvas.style.cursor = hover ? "pointer" : "default";
});
canvas.addEventListener("mousedown", (e) => {
  const p = point(e);
  const n = nodeAt(p.x, p.y);
  moved = 0;
  if (n) dragNode = n;
  else panning = true;
});
window.addEventListener("mouseup", () => { dragNode = null; panning = false; });
canvas.addEventListener("click", (e) => {
  if (moved > 4) return; // itu drag, bukan klik
  const p = point(e);
  const n = nodeAt(p.x, p.y);
  if (n) emit("oxidian-open-note", n.id);
});
canvas.addEventListener("wheel", (e) => {
  e.preventDefault();
  const factor = e.deltaY < 0 ? 1.12 : 0.9;
  const rect = canvas.getBoundingClientRect();
  const mx = e.clientX - rect.left, my = e.clientY - rect.top;
  targetOffset.x = mx - (mx - targetOffset.x) * factor;
  targetOffset.y = my - (my - targetOffset.y) * factor;
  targetScale = Math.max(0.25, Math.min(5, targetScale * factor));
}, { passive: false });

// refresh saat vault berubah (debounce ringan)
let vcTimer = null;
if (listen) {
  listen("vault-changed", () => {
    clearTimeout(vcTimer);
    vcTimer = setTimeout(loadGraph, 800);
  });
}

window.addEventListener("resize", () => {
  heat = Math.max(heat, 0.5); // biarkan fisika menata ulang dengan halus
});

loadGraph().then(() => requestAnimationFrame(draw));
