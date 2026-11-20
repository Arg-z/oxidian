# Oxidian

**Aplikasi pencatatan Markdown yang ditulis 100% di Rust** — kompatibel dengan vault [Obsidian](https://obsidian.md). Data Anda adalah file `.md` biasa; tidak ada format proprietary, tidak ada kode Obsidian yang dipakai (clean-room implementation di atas format vault publik).

![Oxidian](crates/oxidian-app/icons/icon.png)

## Fitur

| Fitur | Status |
|---|---|
| Vault scanner (abaikan `.obsidian/`, `.trash/`, `.git/`, folder tersembunyi) | ✅ |
| Editor Markdown + autosave (atomic write: tmp + rename) | ✅ |
| Preview Markdown (GFM: tabel, code block, blockquote) + sanitasi DOMPurify | ✅ |
| `[[Wikilink]]` / `[[Note\|alias]]` / `[[Note#heading]]` + embed `![[...]]` | ✅ |
| Resolusi link ala Obsidian: path penuh → basename → folder sama → case-insensitive | ✅ |
| Link merah (unresolved) + tawaran buat note saat diklik | ✅ |
| Autocomplete `[[` saat menulis | ✅ |
| Backlinks dengan snippet konteks | ✅ |
| Outgoing links dengan status resolved | ✅ |
| Tags inline `#tag/bersarang` + frontmatter YAML (via gray_matter) | ✅ |
| Full-text search dengan highlight & snippet | ✅ |
| Quick switcher `Ctrl+P` (fuzzy judul/tag/konten) | ✅ |
| Graph view **di jendela terpisah** — force layout, hover, drag node, zoom, klik node → note terbuka di jendela utama | ✅ |
| File watcher — perubahan eksternal otomatis ter-sync | ✅ |
| Rename/move note & folder → **semua link yang menunjuk ikut ditulis ulang** (ala Obsidian), soft-delete ke `.trash/` | ✅ |
| Navigasi `[[Note#Heading]]` → scroll ke heading + highlight | ✅ |
| Tab = indentasi 2 spasi di editor | ✅ |
| Config aplikasi di `~/.config/oxidian/` (vault & note terakhir) — vault tetap Markdown murni | ✅ |

## Arsitektur

```
crates/
├── oxidian-core/      # Library murni (29 unit test)
│   ├── links.rs       # Parser [[wikilink]] & #tag (skip code fence/inline code)
│   ├── vault.rs       # Scan vault, model NoteData, atomic_write, move_to_trash
│   ├── index.rs       # VaultIndex: resolusi link, backlinks (2-pass), tags, graph
│   ├── search.rs      # Scorer fuzzy judul + konten + snippet
│   └── config.rs      # Config JSON di luar vault
└── oxidian-app/       # Shell Tauri 2 (GUI)
    ├── lib.rs         # 19 commands (open_vault, read/write_note, search, graph, ...)
    └── watcher.rs     # notify + debounce, filter event Access (anti-loop)
ui/                   # Frontend statis tanpa bundler
    ├── index.html     # Layout 3-panel ala Obsidian
    ├── app.js         # Logika UI, preview pipeline, autocomplete, graph canvas
    ├── style.css      # Tema gelap aksen ungu
    └── vendor/        # marked.min.js, purify.min.js (di-vendor)
```

**Stack:** Rust 1.99 · Tauri 2 (wry/webkit2gtk) · tanpa Node/bundler — frontend statis di-embed saat build.

## Menjalankan

```bash
# dependensi (Debian/Ubuntu)
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev \
                 librsvg2-dev pkg-config build-essential

# build & run
cargo run

# test
cargo test -p oxidian-core
```

Di VM tanpa GPU, pakai software rendering bila layar hitam:
```bash
WEBKIT_DISABLE_COMPOSITING_MODE=1 WEBKIT_DISABLE_DMABUF_RENDERER=1 cargo run
```

Saat development frontend, agar edit `ui/` langsung terlihat tanpa rebuild:
```bash
python3 -m http.server 8123 --bind 127.0.0.1 -d ui
# set "build": {"devUrl": "http://127.0.0.1:8123"} di crates/oxidian-app/tauri.conf.json
```

## Shortcut

| Shortcut | Aksi |
|---|---|
| `Ctrl+P` | Quick switcher |
| `Ctrl+N` | Note baru |
| `Ctrl+E` | Toggle Edit / Preview |
| `Ctrl+S` | Simpan |
| `Ctrl+G` | Graph view (jendela terpisah) |
| `Esc` | Tutup overlay |

## Kompatibilitas Obsidian

Vault Oxidian **adalah** vault Obsidian: buka folder yang sama di kedua aplikasi dan semuanya bekerja — `.obsidian/` diabaikan (tidak dibaca, tidak ditulis). Yang belum diimplementasi dari Obsidian: plugin system, live-preview WYSIWYG penuh, canvas, sync, mobile.

## Demo

Vault contoh ada di `/data/workspace/vault-demo` — buka lewat tombol *Vault…* atau otomatis saat aplikasi start.
