/* Oxidian frontend — tanpa bundler, jalan di webview Tauri.
 * Semua akses data lewat invoke() ke backend Rust. */
"use strict";

// Tauri v2: invoke ada di namespace core
const invoke = (window.__TAURI__.core && window.__TAURI__.core.invoke) || window.__TAURI__.invoke;
const dialog = window.__TAURI__.dialog || {};
const eventApi = window.__TAURI__.event || {};

/* ---------- helpers DOM ---------- */
const $ = (id) => document.getElementById(id);
const el = (tag, cls, text) => {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
};
const esc = (s) =>
  String(s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));

/* ---------- state ---------- */
const state = {
  vault: null,            // path vault aktif
  notes: new Map(),       // rel -> {title} dari all_notes (cache)
  current: null,          // rel note aktif
  currentRaw: null,       // isi note aktif saat dibuka
  dirty: false,
  mode: "preview",        // "edit" | "preview"
  saveTimer: null,
  lastSelfWrite: { path: null, at: 0 },
  treeOpen: new Set(),    // folder terbuka di tree
  tagMap: null,           // cache tags
  resolvedCache: new Set(), // lowercase path+judul yang valid (untuk warna link)
};

/* ---------- invoke wrapper ---------- */
async function cmd(name, args = {}) {
  try {
    return await invoke(name, args);
  } catch (e) {
    const msg = typeof e === "string" ? e : e?.error || JSON.stringify(e);
    console.error(`${name}:`, msg);
    statusFlash(`⚠ ${msg}`, true);
    throw e;
  }
}

/* ---------- statusbar ---------- */
let statusTimer = null;
function statusFlash(text, isErr = false) {
  const s = $("status-save");
  s.textContent = text;
  s.style.color = isErr ? "var(--danger)" : "var(--text-muted)";
  clearTimeout(statusTimer);
  // error tampil lebih lama supaya sempat dibaca
  statusTimer = setTimeout(() => (s.textContent = ""), isErr ? 10000 : 2500);
}
function setStatus(text) {
  $("status-save").textContent = text;
  $("status-save").style.color = "var(--text-muted)";
}

/* ---------- modal generik ---------- */
let modalResolve = null;
function modal({ title, text = "", input = null }) {
  return new Promise((resolve) => {
    modalResolve = resolve;
    $("modal-title").textContent = title;
    $("modal-text").textContent = text;
    $("modal-text").classList.toggle("hidden", !text);
    const inp = $("modal-input");
    inp.classList.toggle("hidden", input === null);
    if (input !== null) inp.value = input;
    $("modal-overlay").classList.remove("hidden");
    if (input !== null) { inp.focus(); inp.select(); } else $("modal-ok").focus();
  });
}
function closeModal(result) {
  $("modal-overlay").classList.add("hidden");
  if (modalResolve) { modalResolve(result); modalResolve = null; }
}
$("modal-ok").addEventListener("click", () => {
  const inp = $("modal-input");
  closeModal(inp.classList.contains("hidden") ? true : inp.value.trim() || null);
});
$("modal-cancel").addEventListener("click", () => closeModal(null));
$("modal-input").addEventListener("keydown", (e) => {
  if (e.key === "Enter") $("modal-ok").click();
  if (e.key === "Escape") closeModal(null);
});

/* ============================================================
 * BOOT & VAULT
 * ============================================================ */
/* ---------- tema terang/gelap ---------- */
function applyTheme(theme) {
  const t = theme === "light" ? "light" : "dark";
  document.documentElement.dataset.theme = t;
  // label = tema tujuan (aksi), aman untuk semua font
  $("btn-theme").textContent = t === "light" ? "Dark" : "Terang";
  $("btn-theme").title = t === "light" ? "Ganti ke tema gelap" : "Ganti ke tema terang";
  localStorage.setItem("oxidian-theme", t);
}

$("btn-theme").addEventListener("click", () => {
  const next = document.documentElement.dataset.theme === "light" ? "dark" : "light";
  applyTheme(next);
  cmd("set_theme", { theme: next }).catch(() => {});
});

async function boot() {
  const info = await cmd("app_info");
  state.recentVaults = info.recent_vaults || [];
  applyTheme(info.theme);
  if (info.vault) {
    // backend sudah punya vault aktif (tak mungkin saat boot, tapi aman)
    await enterVault(info.vault, info.last_note);
  } else if (state.recentVaults.length) {
    showWelcome(info);
    await tryOpenVault(state.recentVaults[0]); // auto-buka vault terakhir
  } else {
    showWelcome(info);
  }
}

function showWelcome(info) {
  $("welcome").classList.remove("hidden");
  const rec = $("welcome-recent");
  rec.innerHTML = "";
  const recents = info?.recent_vaults?.length ? info.recent_vaults : state.recentVaults || [];
  if (recents.length) {
    const head = el("div", "section-head", "Vault terakhir");
    rec.appendChild(head);
    for (const v of recents.slice(0, 5)) {
      const b = el("button", "recent-item", v);
      b.addEventListener("click", () => tryOpenVault(v));
      rec.appendChild(b);
    }
  }
}

async function tryOpenVault(path) {
  $("welcome-error").classList.add("hidden");
  try {
    const info = await cmd("open_vault", { path });
    state.recentVaults = info.recent_vaults || state.recentVaults;
    await enterVault(info.vault, info.last_note);
  } catch (e) {
    const msg = typeof e === "string" ? e : e?.error || "gagal membuka vault";
    const p = $("welcome-error");
    p.textContent = `Gagal: ${msg}`;
    p.classList.remove("hidden");
  }
}

async function enterVault(vaultPath, lastNote) {
  $("welcome").classList.add("hidden");
  $("app").classList.remove("hidden");
  state.vault = vaultPath;
  state.treeOpen.clear();
  $("vault-name").textContent = vaultPath.split("/").pop();
  $("status-vault").textContent = `${vaultPath}`;
  cmd("app_info").then((i) => {
    $("status-mcp").textContent = i.mcp_port ? `MCP: 127.0.0.1:${i.mcp_port}` : "";
  }).catch(() => {});
  await refreshNotesCache();
  await renderTree();
  await renderTags();
  if (lastNote && state.notes.has(lastNote)) {
    await openNote(lastNote);
  } else {
    const first = [...state.notes.keys()][0];
    if (first) await openNote(first);
  }
}

async function refreshNotesCache() {
  const list = await cmd("all_notes");
  state.notes = new Map(list);
  state.resolvedCache = new Set();
  for (const [rel, title] of list) {
    state.resolvedCache.add(rel.replace(/\.md$/i, "").toLowerCase());
    state.resolvedCache.add(title.toLowerCase());
    const base = rel.split("/").pop().replace(/\.md$/i, "").toLowerCase();
    state.resolvedCache.add(base);
  }
  $("note-count").textContent = String(list.length);
}

/* ---------- tombol welcome ---------- */
$("btn-open-vault").addEventListener("click", async () => {
  const path = await dialog.open?.({ directory: true, multiple: false, title: "Pilih folder vault" });
  if (typeof path === "string") tryOpenVault(path);
});
$("btn-new-vault").addEventListener("click", async () => {
  const parent = await dialog.open?.({ directory: true, multiple: false, title: "Pilih lokasi untuk vault baru" });
  if (typeof parent !== "string") return;
  const name = await modal({ title: "Nama vault baru", input: "Vault Saya" });
  if (!name) return;
  const path = `${parent.replace(/\/$/, "")}/${name}`;
  try {
    await cmd("create_vault", { path });
    await tryOpenVault(path);
  } catch (e) { /* sudah dilaporkan statusFlash */ }
});
$("btn-vault-path-go").addEventListener("click", () => {
  const p = $("vault-path-input").value.trim();
  if (p) tryOpenVault(p);
});
$("vault-path-input").addEventListener("keydown", (e) => {
  if (e.key === "Enter") $("btn-vault-path-go").click();
});
$("btn-open-other").addEventListener("click", () => {
  $("app").classList.add("hidden");
  showWelcome({ recent_vaults: state.recentVaults || [] });
});

/* ============================================================
 * FILE TREE
 * ============================================================ */
async function renderTree() {
  const tree = await cmd("tree");
  const root = $("file-tree");
  root.innerHTML = "";
  root.appendChild(buildTreeNodes(tree, ""));
}

function buildTreeNodes(nodes, prefix) {
  const wrap = el("div", "tree-children");
  for (const n of nodes) {
    const item = el("div", "tree-item");
    item.dataset.path = n.path;
    if (n.type === "dir") {
      const open = state.treeOpen.has(n.path);
      const caret = el("span", "caret" + (open ? " open" : ""), "▸");
      item.appendChild(caret);
      item.appendChild(el("span", "label", n.name));
      const acts = el("div", "row-actions");
      const bNew = el("button", null, "＋");
      bNew.title = "Note baru di folder ini";
      bNew.addEventListener("click", (e) => { e.stopPropagation(); newNoteIn(n.path); });
      const bRen = el("button", null, "✎");
      bRen.title = "Ganti nama folder";
      bRen.addEventListener("click", (e) => { e.stopPropagation(); renameItem(n); });
      const bDel = el("button", null, "🗑");
      bDel.title = "Hapus folder";
      bDel.addEventListener("click", (e) => { e.stopPropagation(); deleteItem(n); });
      acts.append(bNew, bRen, bDel);
      item.appendChild(acts);
      item.addEventListener("click", () => {
        state.treeOpen.has(n.path) ? state.treeOpen.delete(n.path) : state.treeOpen.add(n.path);
        renderTree();
      });
      wrap.appendChild(item);
      if (open) wrap.appendChild(buildTreeNodes(n.children, n.path + "/"));
    } else {
      item.appendChild(el("span", "caret", ""));
      item.appendChild(el("span", "label", n.name));
      const acts = el("div", "row-actions");
      const bRen = el("button", null, "✎");
      bRen.addEventListener("click", (e) => { e.stopPropagation(); renameItem(n); });
      const bDel = el("button", null, "🗑");
      bDel.addEventListener("click", (e) => { e.stopPropagation(); deleteItem(n); });
      acts.append(bRen, bDel);
      item.appendChild(acts);
      if (state.current === n.path) item.classList.add("active");
      item.addEventListener("click", () => openNote(n.path));
      wrap.appendChild(item);
    }
  }
  wrap.classList.toggle("hidden", prefix === "" ? false : false);
  return wrap;
}
// (helper: folder level-0 selalu tampil)

async function renameItem(n) {
  const isFile = n.type === "file";
  const base = isFile ? n.name.replace(/\.md$/i, "") : n.name;
  const newName = await modal({ title: `Ganti nama ${isFile ? "note" : "folder"}`, input: base, text: n.path });
  if (!newName || newName === base) return;
  const parent = n.path.split("/").slice(0, -1).join("/");
  const target = parent ? `${parent}/${newName}` : newName;
  try {
    await cmd("rename_path", { from: n.path, to: isFile ? target + ".md" : target });
    if (state.current === n.path) state.current = null;
    await refreshNotesCache();
    await renderTree();
    await renderTags();
    if (isFile && state.current === null) openNote(target + ".md");
    statusFlash("✓ diganti nama");
  } catch (e) { /* dilaporkan */ }
}

async function deleteItem(n) {
  const ok = await modal({ title: `Hapus ${n.type === "file" ? "note" : "folder"}?`, text: `${n.path} → dipindah ke .trash/` });
  if (!ok) return;
  await cmd("delete_path", { rel: n.path });
  if (state.current === n.path || (state.current && state.current.startsWith(n.path + "/"))) {
    state.current = null;
    closeNoteView();
  }
  await refreshNotesCache();
  await renderTree();
  await renderTags();
  statusFlash("✓ dipindah ke .trash");
}

/* ============================================================
 * NOTE: buka / simpan / tutup
 * ============================================================ */
async function openNote(rel) {
  await flushSave();
  let note;
  try {
    note = await cmd("read_note", { rel });
  } catch (e) {
    statusFlash(`⚠ gagal membuka ${rel}`, true);
    return;
  }
  state.current = note.rel_path;
  state.currentRaw = note.content;
  state.lastFm = note.frontmatter || {};
  state.dirty = false;
  $("note-title").textContent = note.title;
  $("tab-path").textContent = note.rel_path !== note.title + ".md" ? note.rel_path : "";
  $("empty-state").classList.add("hidden");
  applyMode();
  $("editor").value = note.content;
  renderPreview(note);
  renderBacklinks(note.rel_path);
  renderOutlinks(note);
  renderProperties(note.frontmatter);
  await expandToNote(note.rel_path); // buka foldernya supaya highlight terlihat
  markActiveInTree();
  cmd("set_last_note", { rel: note.rel_path }).catch(() => {});
}

/// Pastikan semua folder induk `rel` ter-expand di tree.
async function expandToNote(rel) {
  const segs = rel.split("/");
  let added = false;
  for (let i = 1; i < segs.length; i++) {
    const dir = segs.slice(0, i).join("/");
    if (!state.treeOpen.has(dir)) {
      state.treeOpen.add(dir);
      added = true;
    }
  }
  if (added) await renderTree();
}

function markActiveInTree() {
  document.querySelectorAll(".tree-item.active").forEach((n) => n.classList.remove("active"));
  if (!state.current) return;
  document.querySelectorAll(".tree-item").forEach((item) => {
    if (item.dataset?.path === state.current) item.classList.add("active");
  });
}

function closeNoteView() {
  state.current = null;
  state.currentRaw = null;
  $("note-title").textContent = "—";
  $("tab-path").textContent = "";
  $("editor").value = "";
  $("preview").innerHTML = "";
  $("empty-state").classList.remove("hidden");
  $("backlinks").innerHTML = "<span class='muted'>—</span>";
  $("outlinks").innerHTML = "<span class='muted'>—</span>";
  $("properties").innerHTML = "<span class='muted'>—</span>";
}

/* ---------- simpan ---------- */
function scheduleSave() {
  if (!state.current || state.mode !== "edit") return;
  state.dirty = true;
  setStatus("○ menyimpan…");
  clearTimeout(state.saveTimer);
  state.saveTimer = setTimeout(flushSave, 900);
}

async function flushSave() {
  clearTimeout(state.saveTimer);
  if (!state.current || !state.dirty) return;
  const rel = state.current;
  const content = $("editor").value;
  state.dirty = false;
  state.lastSelfWrite = { path: rel, at: Date.now() };
  try {
    const note = await cmd("write_note", { rel, content });
    state.currentRaw = note.content;
    if (state.current === rel) {
      renderBacklinks(rel);
      renderOutlinks(note);
      renderProperties(note.frontmatter);
    }
    // refresh cache judul bila note baru
    if (!state.notes.has(note.rel_path)) await refreshNotesCache();
    setStatus("✓ tersimpan");
  } catch (e) {
    state.dirty = true;
  }
}

$("editor").addEventListener("input", () => { scheduleSave(); maybeShowWikibox(); });
$("editor").addEventListener("keydown", editorKeydown);
$("editor").addEventListener("click", () => hideWikibox());
$("editor").addEventListener("blur", () => setTimeout(() => hideWikibox(), 150));

/* ---------- mode edit / preview ---------- */
/// mainkan animasi fade-up pada elemen yang baru tampil
function playFade(el2) {
  if (!el2) return;
  el2.classList.remove("note-in");
  void el2.offsetWidth; // restart animation
  el2.classList.add("note-in");
}

function applyMode() {
  const editing = state.mode === "edit" && state.current;
  const editor = $("editor"), preview = $("preview");
  editor.classList.toggle("hidden", !editing);
  preview.classList.toggle("hidden", state.current ? editing : false);
  $("btn-mode").textContent = editing ? "👁 Preview" : "✏ Edit";
  playFade(editing ? editor : preview);
  if (editing) { editor.focus(); hideWikibox(); }
}
$("btn-mode").addEventListener("click", () => switchMode());

function switchMode() {
  if (!state.current) return;
  state.mode = state.mode === "edit" ? "preview" : "edit";
  if (state.mode === "preview") {
    renderPreview({ content: $("editor").value, frontmatter: state.lastFm || {} });
  }
  applyMode();
}

/* ============================================================
 * PREVIEW MARKDOWN + WIKILINK
 * ============================================================ */
marked.setOptions({ gfm: true, breaks: true });

function isResolvedTarget(target) {
  const t = target.replace(/\.md$/i, "").toLowerCase();
  if (state.resolvedCache.has(t)) return true;
  const base = t.split("/").pop();
  return state.resolvedCache.has(base);
}

/// Ganti [[wikilink]] menjadi anchor HTML — hanya di luar code fence & inline code.
function preprocessWikilinks(md) {
  const lines = md.split("\n");
  let fence = false;
  const re = /(!?)\[\[([^\]|\n#]+)(?:#([^\]|\n]+))?(?:\|([^\]\n]*))?\]\]/g;
  const out = lines.map((line) => {
    const t = line.trimStart();
    if (t.startsWith("```") || t.startsWith("~~~")) { fence = !fence; return line; }
    if (fence) return line;
    // skip inline code segments
    const parts = line.split("`");
    // parts index genap = di luar inline code
    for (let i = 0; i < parts.length; i += 2) {
      parts[i] = parts[i].replace(re, (m, embed, target, heading, alias) => {
        const tgt = target.trim();
        const head = heading ? heading.trim() : null;
        const disp = (alias && alias.trim()) || tgt + (head ? "#" + head : "");
        const cls = isResolvedTarget(tgt) ? "wikilink" : "wikilink unresolved";
        const dh = head ? ` data-heading="${esc(head)}"` : "";
        if (embed) {
          return `<a class="${cls} embed" data-target="${esc(tgt)}"${dh} href="javascript:void(0)">📎 ${esc(disp)}</a>`;
        }
        return `<a class="${cls}" data-target="${esc(tgt)}"${dh} href="javascript:void(0)">${esc(disp)}</a>`;
      });
    }
    return parts.join("`");
  });
  return out.join("\n");
}

function renderPreview(note) {
  const prev = $("preview");
  if (!state.current) return;
  let html = "";
  // properties block
  const fm = note.frontmatter || {};
  const keys = Object.keys(fm);
  if (keys.length && state.mode !== "edit") {
    const rows = keys.map((k) => `<div class="fm-row"><b>${esc(k)}</b> ${esc(JSON.stringify(fm[k]).replace(/^"|"$/g, ""))}</div>`).join("");
    html += `<div class="fm-block">${rows}</div>`;
  }
  const md = preprocessWikilinks(note.content.startsWith("---") ? stripFm(note.content) : note.content);
  const parsed = marked.parse(md);
  html += DOMPurify.sanitize(parsed, { ADD_ATTR: ["data-target", "data-heading"] });
  // bungkus agar konten ter-center dengan lebar baca nyaman
  prev.innerHTML = `<div class="preview-inner">${html}</div>`;
  prev.querySelectorAll("a.wikilink").forEach((a) => {
    a.addEventListener("click", () => followLink(a.dataset.target, a.dataset.heading || null));
  });
}

function stripFm(content) {
  if (!content.startsWith("---")) return content;
  const end = content.indexOf("\n---", 3);
  return end === -1 ? content : content.slice(end + 4).replace(/^\n/, "");
}

async function followLink(target, heading = null) {
  const from = state.current || "";
  let rel = null;
  try {
    rel = await cmd("resolve_link", { from, target });
  } catch (e) { return; }
  if (rel) {
    await openNote(rel);
    if (heading) scrollPreviewToHeading(heading);
    return;
  }
  const ok = await modal({
    title: "Note tidak ditemukan",
    text: `Buat note baru "${target}"?`,
  });
  if (!ok) return;
  const relNew = await ensureNote(target);
  if (relNew) {
    await refreshNotesCache();
    await renderTree();
    await openNote(relNew);
    if (heading) scrollPreviewToHeading(heading);
  }
}

/// Gulir preview ke heading yang cocok (untuk link [[Note#Heading]]).
function scrollPreviewToHeading(heading) {
  if (state.mode !== "preview") return;
  const prev = $("preview");
  const want = heading.trim().toLowerCase();
  const hs = prev.querySelectorAll("h1, h2, h3, h4, h5, h6");
  let el2 = null;
  for (const h of hs) {
    if (h.textContent.trim().toLowerCase() === want) { el2 = h; break; }
  }
  if (!el2) {
    for (const h of hs) {
      if (h.textContent.trim().toLowerCase().includes(want)) { el2 = h; break; }
    }
  }
  if (el2) {
    el2.scrollIntoView({ block: "start", behavior: "smooth" });
    el2.classList.remove("heading-flash");
    // restart animasi
    void el2.offsetWidth;
    el2.classList.add("heading-flash");
  } else {
    statusFlash(`heading "${heading}" tidak ditemukan`, true);
  }
}

async function ensureNote(target) {
  // buat note baru; kalau ada folder di target, backend yang bikin foldernya
  const body = `# ${target.split("/").pop()}\n\n`;
  try {
    await cmd("create_note", { rel: target, content: body });
    return target.endsWith(".md") ? target : target + ".md";
  } catch (e) {
    return null;
  }
}

/* ============================================================
 * WIKILINK AUTOCOMPLETE
 * ============================================================ */
let wikiState = null; // { start, query }

function maybeShowWikibox() {
  const ta = $("editor");
  const pos = ta.selectionStart;
  const text = ta.value.slice(0, pos);
  const m = text.match(/\[\[([^\]|\n]*)$/);
  if (!m) { hideWikibox(); return; }
  wikiState = { start: pos - m[1].length - 2, query: m[1] };
  showWikibox(m[1]);
}

function showWikibox(query) {
  const box = $("wikibox");
  const q = query.toLowerCase();
  const entries = [...state.notes.entries()].map(([rel, title]) => ({ rel, title }));
  let items = entries.filter((e) =>
    e.title.toLowerCase().includes(q) || e.rel.toLowerCase().includes(q));
  items = items.slice(0, 8);
  const showNew = q && !isResolvedTarget(q) && !items.some((e) => e.title.toLowerCase() === q);
  box.innerHTML = "";
  if (!items.length && !showNew) {
    box.appendChild(el("div", "wiki-empty", "Tidak ada note yang cocok"));
  }
  items.forEach((it, i) => {
    const d = el("div", "wiki-item" + (i === 0 ? " sel" : ""));
    d.appendChild(el("span", "t", it.title));
    d.appendChild(el("span", "p", it.rel));
    d.dataset.rel = it.rel;
    d.addEventListener("mousedown", (e) => { e.preventDefault(); pickWiki(it); });
    box.appendChild(d);
  });
  if (showNew) {
    const d = el("div", "wiki-item wiki-new" + (items.length ? "" : " sel"));
    d.appendChild(el("span", "t", `+ Buat "${query}"`));
    d.dataset.rel = query;
    d.addEventListener("mousedown", (e) => { e.preventDefault(); pickWiki({ rel: query, isNew: true }); });
    box.appendChild(d);
  }
  box.classList.remove("hidden");
  positionWikibox();
  box._items = items.length ? items : [{ rel: query, isNew: true }];
  box._sel = 0;
}

function positionWikibox() {
  const ta = $("editor");
  const box = $("wikibox");
  // mirror measurement
  const mirror = document.createElement("div");
  const cs = getComputedStyle(ta);
  for (const prop of ["fontSize", "fontFamily", "fontWeight", "lineHeight", "padding", "whiteSpace", "wordWrap", "letterSpacing", "boxSizing", "width"]) {
    mirror.style[prop] = cs[prop];
  }
  mirror.style.position = "absolute";
  mirror.style.visibility = "hidden";
  mirror.style.height = "auto";
  mirror.style.overflowWrap = "break-word";
  document.body.appendChild(mirror);
  const before = ta.value.slice(0, ta.selectionStart);
  mirror.textContent = before;
  const marker = document.createElement("span");
  marker.textContent = "\u200b";
  mirror.appendChild(marker);
  const mr = marker.getBoundingClientRect();
  const tr = ta.getBoundingClientRect();
  const editorRect = ta.parentElement.getBoundingClientRect();
  let x = mr.left - tr.left + parseFloat(cs.paddingLeft) + (editorRect.left);
  let y = mr.top - tr.top + parseFloat(cs.paddingTop) + editorRect.top + parseFloat(cs.lineHeight || "22px") - ta.scrollTop;
  document.body.removeChild(mirror);
  x = Math.min(x, editorRect.right - 280);
  y = Math.min(y, window.innerHeight - 260);
  box.style.left = `${Math.max(8, x)}px`;
  box.style.top = `${Math.max(8, y)}px`;
}

function hideWikibox() {
  $("wikibox").classList.add("hidden");
  wikiState = null;
}

function editorKeydown(e) {
  const box = $("wikibox");
  if (!box.classList.contains("hidden")) {
    const items = box._items || [];
    if (e.key === "ArrowDown" || e.key === "ArrowUp" || e.key === "Tab") {
      e.preventDefault();
      const dir = e.key === "ArrowUp" ? -1 : 1;
      box._sel = (box._sel + dir + items.length) % items.length;
      box.querySelectorAll(".wiki-item").forEach((n, i) => n.classList.toggle("sel", i === box._sel));
      return;
    }
    if (e.key === "Enter" || e.key === "Tab") {
      e.preventDefault();
      const it = items[box._sel];
      if (it) pickWiki(it);
      return;
    }
    if (e.key === "Escape") { hideWikibox(); return; }
  }
  if (e.key === "Tab" && !e.ctrlKey && !e.metaKey && !e.shiftKey) {
    // Tab di editor → indentasi 2 spasi (bukan pindah fokus)
    e.preventDefault();
    const ta = $("editor");
    ta.setRangeText("  ", ta.selectionStart, ta.selectionEnd, "end");
    scheduleSave();
    return;
  }
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
    e.preventDefault();
    flushSave().then(() => statusFlash("✓ tersimpan"));
  }
}

async function pickWiki(it) {
  const ta = $("editor");
  const pos = ta.selectionStart;
  const after = ta.value.slice(pos);
  let insert = `${it.rel}]]`;
  if (after.startsWith("]]")) insert = it.rel;
  ta.setRangeText(insert, wikiState.start, pos, "end");
  if (it.isNew) {
    // buat note segera supaya link resolve
    const relNew = await ensureNote(it.rel);
    if (relNew) {
      await refreshNotesCache();
      await renderTree();
      await renderTags();
    }
  }
  hideWikibox();
  ta.focus();
  scheduleSave();
}

/* ============================================================
 * BACKLINKS / OUTLINKS / PROPERTIES
 * ============================================================ */
async function renderBacklinks(rel) {
  const wrap = $("backlinks");
  const items = await cmd("backlinks", { rel });
  wrap.innerHTML = "";
  if (!items.length) {
    wrap.appendChild(el("span", "muted", "Belum ada yang menaut."));
    return;
  }
  for (const it of items) {
    const d = el("div", "link-item");
    d.appendChild(el("div", "li-title", it.title));
    if (it.snippet) d.appendChild(el("div", "li-snippet", it.snippet));
    d.addEventListener("click", () => openNote(it.rel_path));
    wrap.appendChild(d);
  }
}

function renderOutlinks(note) {
  const wrap = $("outlinks");
  wrap.innerHTML = "";
  if (!note.outgoing_links?.length) {
    wrap.appendChild(el("span", "muted", "Tidak ada link keluar."));
    return;
  }
  const seen = new Set();
  for (const l of note.outgoing_links) {
    if (seen.has(l.target)) continue;
    seen.add(l.target);
    const d = el("div", "link-item" + (l.resolved ? "" : " resolved-no"));
    d.appendChild(el("div", "li-title", `${l.resolved ? "🔗" : "🔗?"} ${l.target}`));
    d.addEventListener("click", () => followLink(l.target));
    wrap.appendChild(d);
  }
}

function renderProperties(fm) {
  const wrap = $("properties");
  wrap.innerHTML = "";
  const keys = Object.keys(fm || {});
  if (!keys.length) {
    wrap.appendChild(el("span", "muted", "Tanpa frontmatter."));
    return;
  }
  for (const k of keys) {
    const row = el("div", "prop-row");
    row.appendChild(el("span", "prop-key", k));
    row.appendChild(el("span", "prop-val", JSON.stringify(fm[k])));
    wrap.appendChild(row);
  }
}

/* ============================================================
 * SEARCH SIDEBAR
 * ============================================================ */
let searchTimer = null;
$("search-input").addEventListener("input", () => {
  clearTimeout(searchTimer);
  searchTimer = setTimeout(runSearch, 250);
});

async function runSearch() {
  const q = $("search-input").value.trim();
  const box = $("search-results");
  if (!q) { box.classList.add("hidden"); box.innerHTML = ""; return; }
  const hits = await cmd("search_notes", { query: q });
  box.innerHTML = "";
  box.classList.remove("hidden");
  for (const h of hits.slice(0, 30)) {
    const d = el("div", "search-hit");
    d.appendChild(el("div", "hit-title", h.title));
    const s = el("div", "hit-snippet");
    s.innerHTML = esc(h.snippet || h.rel_path).replace(
      new RegExp(`(${q.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")})`, "ig"), "<mark>$1</mark>");
    d.appendChild(s);
    d.addEventListener("click", () => { openNote(h.rel_path); box.classList.add("hidden"); });
    box.appendChild(d);
  }
  if (!hits.length) box.appendChild(el("div", "qs-empty", "Tidak ada hasil"));
}

/* ============================================================
 * TAGS
 * ============================================================ */
$("tags-toggle").addEventListener("click", () => {
  $("tag-list").classList.toggle("hidden");
  $("tags-toggle").classList.toggle("open");
});

async function renderTags() {
  const tags = await cmd("all_tags");
  state.tagMap = new Map(tags);
  $("tag-count").textContent = String(tags.length);
  const box = $("tag-list");
  box.innerHTML = "";
  for (const [t, n] of tags) {
    const chip = el("span", "tag-chip");
    chip.textContent = `#${t} `;
    const b = el("b", null, String(n));
    chip.appendChild(b);
    chip.addEventListener("click", async () => {
      const notes = await cmd("notes_with_tag", { tag: t });
      showTagResults(t, notes);
    });
    box.appendChild(chip);
  }
}

async function showTagResults(tag, notes) {
  const box = $("search-results");
  box.innerHTML = "";
  box.classList.remove("hidden");
  box.appendChild(el("div", "qs-empty", `Tag #${tag} — ${notes.length} note`));
  for (const rel of notes) {
    const d = el("div", "search-hit");
    d.appendChild(el("div", "hit-title", state.notes.get(rel) || rel));
    d.appendChild(el("div", "hit-snippet", rel));
    d.addEventListener("click", () => { openNote(rel); box.classList.add("hidden"); });
    box.appendChild(d);
  }
}

/* ============================================================
 * QUICK SWITCHER
 * ============================================================ */
function openQuickSwitcher() {
  $("qs-overlay").classList.remove("hidden");
  const inp = $("qs-input");
  inp.value = "";
  renderQs("");
  inp.focus();
}
function closeQuickSwitcher() {
  $("qs-overlay").classList.add("hidden");
}
$("qs-input").addEventListener("input", (e) => renderQs(e.target.value));

let qsItems = [];
let qsSel = 0;

async function renderQs(q) {
  const hits = await cmd("search_notes", { query: q });
  const box = $("qs-results");
  box.innerHTML = "";
  qsItems = hits.slice(0, 12);
  qsSel = 0;
  if (!hits.length) {
    box.appendChild(el("div", "qs-empty", q ? `Buat note baru "${q}" (Enter)` : "Vault masih kosong — buat note dengan Ctrl+N"));
    box._newTitle = q || null;
    return;
  }
  box._newTitle = null;
  qsItems.forEach((h, i) => {
    const d = el("div", "qs-item" + (i === 0 ? " sel" : ""));
    d.appendChild(el("span", "t", h.title));
    d.appendChild(el("span", "p", h.rel_path + (h.snippet ? " — " + h.snippet.slice(0, 60) : "")));
    d.addEventListener("click", () => { openNote(h.rel_path); closeQuickSwitcher(); });
    box.appendChild(d);
  });
}

$("qs-input").addEventListener("keydown", (e) => {
  const box = $("qs-results");
  if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    e.preventDefault();
    qsSel = (qsSel + (e.key === "ArrowUp" ? -1 : 1) + qsItems.length) % Math.max(1, qsItems.length);
    box.querySelectorAll(".qs-item").forEach((n, i) => n.classList.toggle("sel", i === qsSel));
  } else if (e.key === "Enter") {
    e.preventDefault();
    const chosen = qsItems[qsSel];
    if (chosen) {
      openNote(chosen.rel_path);
      closeQuickSwitcher();
    } else if (box._newTitle) {
      followLink(box._newTitle);
      closeQuickSwitcher();
    }
  } else if (e.key === "Escape") {
    closeQuickSwitcher();
  }
});
$("qs-overlay").addEventListener("mousedown", (e) => {
  if (e.target === $("qs-overlay")) closeQuickSwitcher();
});

/* ============================================================
 * GRAPH VIEW — jendela terpisah (implementasi di ui/graph.js)
 * ============================================================ */
$("btn-graph").addEventListener("click", () => cmd("open_graph_window").catch(() => {}));

// node diklik di graph window → buka note di jendela utama
if (eventApi.listen) {
  eventApi.listen("oxidian-open-note", (ev) => {
    const rel = ev.payload;
    if (typeof rel === "string" && rel) {
      state.mode = "preview";
      openNote(rel);
    }
  });
}

/* ============================================================
 * NOTE BARU / FOLDER BARU
 * ============================================================ */
$("btn-new-note").addEventListener("click", () => newNoteIn(null));
async function newNoteIn(folder) {
  if (!state.vault) return;
  const name = await modal({
    title: "Note baru",
    input: folder ? `${folder}/judul` : "judul",
    text: "Boleh pakai / untuk folder, mis: Proyek/Ide Baru",
  });
  if (!name) return;
  const rel = name.endsWith(".md") ? name : name + ".md";
  try {
    await cmd("create_note", { rel, content: `# ${rel.replace(/\.md$/, "").split("/").pop()}\n\n` });
    await refreshNotesCache();
    await renderTree();
    state.mode = "edit";
    await openNote(rel);
    $("editor").focus();
    statusFlash("✓ note dibuat");
  } catch (e) { /* dilaporkan */ }
}

$("btn-new-folder").addEventListener("click", async () => {
  if (!state.vault) return;
  const name = await modal({ title: "Folder baru", input: "Nama Folder" });
  if (!name) return;
  await cmd("create_folder", { rel: name });
  await renderTree();
  statusFlash("✓ folder dibuat");
});

/* ============================================================
 * EVENT VAULT-CHANGED (watcher eksternal)
 * ============================================================ */
let vcTimer = null;
if (eventApi.listen) {
  eventApi.listen("vault-changed", (ev) => {
    const payload = ev.payload || {};
    const paths = payload.paths || [];
    const isSelfWrite = payload.self === true && Date.now() - state.lastSelfWrite.at < 2500;
    clearTimeout(vcTimer);
    vcTimer = setTimeout(async () => {
      // catatan: write_note sendiri juga memicu refresh di sisi backend.
      await cmd("rescan").catch(() => {});
      await refreshNotesCache();
      await renderTree();
      await renderTags();
      if (state.current && !isSelfWrite && paths.includes(state.current) && !state.dirty) {
        const note = await cmd("read_note", { rel: state.current });
        state.currentRaw = note.content;
        $("editor").value = note.content;
        renderPreview(note);
        renderBacklinks(state.current);
        renderOutlinks(note);
        statusFlash("↻ diperbarui dari disk");
      }
    }, 400);
  });
}

/* ============================================================
 * SHORTCUTS GLOBAL
 * ============================================================ */
window.addEventListener("keydown", (e) => {
  const ctrl = e.ctrlKey || e.metaKey;
  if (ctrl && e.key.toLowerCase() === "p") { e.preventDefault(); openQuickSwitcher(); }
  else if (ctrl && e.key.toLowerCase() === "e" && state.current) {
    e.preventDefault();
    switchMode();
  }
  else if (ctrl && e.key.toLowerCase() === "s") { e.preventDefault(); flushSave().then(() => statusFlash("✓ tersimpan")); }
  else if (ctrl && e.key.toLowerCase() === "n") { e.preventDefault(); newNoteIn(null); }
  else if (ctrl && e.key.toLowerCase() === "g") { e.preventDefault(); cmd("open_graph_window").catch(() => {}); }
  else if (e.key === "Escape") {
    if (!$("qs-overlay").classList.contains("hidden")) closeQuickSwitcher();
    else if (!$("modal-overlay").classList.contains("hidden")) closeModal(null);
    else if (!$("search-results").classList.contains("hidden")) $("search-results").classList.add("hidden");
  }
});

/* klik pada area preview juga bisa Ctrl+klik utk edit? skip. */
$("preview").addEventListener("dblclick", () => {
  if (state.current) { state.mode = "edit"; applyMode(); }
});

/* pastikan tidak ada perubahan yang hilang saat window ditutup / disembunyikan */
window.addEventListener("beforeunload", () => { flushSave(); });
document.addEventListener("visibilitychange", () => {
  if (document.visibilityState === "hidden") flushSave();
});

boot().catch((e) => {
  console.error("boot error", e);
});
