//! Oxidian app: shell Tauri di atas [`oxidian_core`].
//! Semua akses vault lewat commands di sini; UI (HTML/JS statis) manggil via `invoke`.

mod mcp;
mod watcher;

use oxidian_core::{config::Config, search, vault, VaultIndex};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};

/// State yang dibagi antara GUI (commands) & MCP server — semua akses via Mutex.
pub struct AppStateInner {
    pub(crate) vault: Mutex<Option<ActiveVault>>,
    pub(crate) config: Mutex<Config>,
}

/// Alias: state dibungkus Arc supaya bisa di-clone ke thread MCP.
pub type AppState = Arc<AppStateInner>;

pub(crate) struct ActiveVault {
    pub(crate) root: PathBuf,
    pub(crate) index: VaultIndex,
    watcher_stop: Option<std::sync::mpsc::SyncSender<()>>,
}

impl Drop for ActiveVault {
    fn drop(&mut self) {
        if let Some(tx) = &self.watcher_stop {
            let _ = tx.send(());
        }
    }
}

#[derive(Serialize)]
struct ErrorJson {
    error: String,
}

fn err<E: std::fmt::Display>(e: E) -> ErrorJson {
    ErrorJson {
        error: e.to_string(),
    }
}

// ---------- tipe serialisasi ke frontend ----------

#[derive(Serialize)]
struct AppInfo {
    vault: Option<String>,
    recent_vaults: Vec<String>,
    last_note: Option<String>,
    note_count: usize,
    mcp_port: Option<u16>,
    theme: String,
}

#[derive(Serialize, Clone)]
struct TreeLeaf {
    #[serde(rename = "type")]
    kind: &'static str, // "file" | "dir"
    path: String,
    name: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    children: Vec<TreeLeaf>,
}

#[derive(Serialize)]
struct NotePayload {
    rel_path: String,
    title: String,
    content: String,
    tags: Vec<String>,
    frontmatter: serde_json::Value,
    outgoing_links: Vec<OutLink>,
}

#[derive(Serialize)]
struct OutLink {
    target: String,
    resolved: Option<String>,
}

#[derive(Serialize)]
struct BacklinkItem {
    rel_path: String,
    title: String,
    snippet: String,
}

#[derive(Serialize)]
struct GraphPayload {
    nodes: Vec<oxidian_core::index::GraphNode>,
    edges: Vec<oxidian_core::index::Edge>,
}

// ---------- helpers ----------

fn with_vault<T>(
    state: &State<AppState>,
    f: impl FnOnce(&ActiveVault) -> Result<T, String>,
) -> Result<T, ErrorJson> {
    let guard = state.vault.lock().unwrap();
    match guard.as_ref() {
        Some(v) => f(v).map_err(err),
        None => Err(err("belum ada vault terbuka")),
    }
}

/// Bangun tree dari daftar rel_path note (folder dirakit dari path).
fn build_tree(paths: &[String]) -> Vec<TreeLeaf> {
    #[derive(Default)]
    struct Dir {
        dirs: std::collections::BTreeMap<String, Dir>,
        files: Vec<TreeLeaf>,
    }
    fn insert(dir: &mut Dir, rel: &str, prefix: &str) {
        let (head, rest) = match rel.split_once('/') {
            Some((h, r)) => (h, Some(r)),
            None => (rel, None),
        };
        match rest {
            Some(r) => insert(
                dir.dirs.entry(head.to_string()).or_default(),
                r,
                &format!("{prefix}{head}/"),
            ),
            None => dir.files.push(TreeLeaf {
                kind: "file",
                path: format!("{prefix}{rel}"), // path PENUH dari root vault
                name: vault::title_of(rel),
                children: Vec::new(),
            }),
        }
    }
    /// Rakit subtree: folder jadi node ber-children, diikuti file di levelnya.
    fn to_leaves(dir: &Dir, path_prefix: &str) -> Vec<TreeLeaf> {
        let mut out: Vec<TreeLeaf> = dir
            .dirs
            .iter()
            .map(|(n, d)| TreeLeaf {
                kind: "dir",
                path: format!("{path_prefix}{n}"),
                name: n.clone(),
                children: to_leaves(d, &format!("{path_prefix}{n}/")),
            })
            .collect();
        out.extend(dir.files.clone());
        out
    }
    let mut root = Dir::default();
    for p in paths {
        insert(&mut root, p, "");
    }
    to_leaves(&root, "")
}

// ---------- commands ----------

/// Info app — satu implementasi untuk command `app_info` & hasil `open_vault`.
fn app_info_of(state: &State<AppState>) -> AppInfo {
    let cfg = state.config.lock().unwrap();
    let (v, n) = {
        let guard = state.vault.lock().unwrap();
        match guard.as_ref() {
            Some(v) => (Some(v.root.display().to_string()), v.index.len()),
            None => (None, 0),
        }
    };
    AppInfo {
        vault: v,
        recent_vaults: cfg.recent_vaults.clone(),
        last_note: cfg.last_note.clone(),
        note_count: n,
        mcp_port: cfg.mcp_port,
        theme: cfg.theme.clone(),
    }
}

#[tauri::command]
fn app_info(state: State<AppState>) -> AppInfo {
    app_info_of(&state)
}

#[tauri::command]
fn open_vault(app: AppHandle, state: State<AppState>, path: String) -> Result<AppInfo, ErrorJson> {
    let root = PathBuf::from(&path);
    let notes = vault::scan_vault(&root).map_err(err)?;
    let index = VaultIndex::build(notes);

    // matikan watcher lama (Drop mengirim sinyal stop)
    let stop = watcher::spawn(&app, &root);
    {
        let mut guard = state.vault.lock().unwrap();
        *guard = Some(ActiveVault {
            root: root.clone(),
            index,
            watcher_stop: stop,
        });
    }
    {
        let mut cfg = state.config.lock().unwrap();
        cfg.push_recent_vault(&path);
        cfg.last_note = None;
        let _ = cfg.save();
    }
    Ok(app_info_of(&state))
}

#[tauri::command]
fn tree(state: State<AppState>) -> Result<Vec<TreeLeaf>, ErrorJson> {
    with_vault(&state, |v| Ok(build_tree(&v.index.paths())))
}

/// Buat folder vault baru + satu note pembuka (dipakai tombol "Buat Vault Baru").
#[tauri::command]
fn create_vault(path: String) -> Result<String, ErrorJson> {
    let root = PathBuf::from(&path);
    std::fs::create_dir_all(&root)
        .map_err(|e| e.to_string())
        .map_err(err)?;
    let starter = root.join("Selamat Datang.md");
    if !starter.exists() {
        vault::atomic_write(
            &starter,
            "# Selamat Datang di Oxidian\n\nVault ini hanyalah folder berisi file Markdown biasa —\nbisa dibuka Obsidian asli atau editor apa pun.\n\nCoba:\n- Tekan `Ctrl+P` untuk quick switcher\n- Buat note baru dengan `Ctrl+N`\n- Tulis [[Note Baru]] untuk membuat wikilink\n- Tambahkan #oxidian #demo sebagai tag\n",
        )
        .map_err(|e| e.to_string())
        .map_err(err)?;
    }
    Ok(root.display().to_string())
}

#[tauri::command]
fn read_note(state: State<AppState>, rel: String) -> Result<NotePayload, ErrorJson> {
    with_vault(&state, |v| {
        let rel = vault::normalize_rel(&rel);
        let path = v.root.join(&rel);
        let raw = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let note = vault::parse_note(&rel, &raw);
        let outgoing = note
            .links
            .iter()
            .map(|l| OutLink {
                target: l.target.clone(),
                resolved: match v.index.resolve(&rel, &l.target) {
                    oxidian_core::index::Resolved::Note(p) => Some(p),
                    oxidian_core::index::Resolved::Unresolved(_) => None,
                },
            })
            .collect();
        let payload = NotePayload {
            rel_path: note.rel_path,
            title: note.title,
            content: raw,
            tags: note.tags,
            frontmatter: serde_json::to_value(&note.frontmatter).unwrap_or_default(),
            outgoing_links: outgoing,
        };
        Ok(payload)
    })
}

#[tauri::command]
fn write_note(
    app: AppHandle,
    state: State<AppState>,
    rel: String,
    content: String,
) -> Result<NotePayload, ErrorJson> {
    let rel = vault::ensure_note_path(&rel).map_err(err)?;
    {
        let guard = state.vault.lock().unwrap();
        let v = guard.as_ref().ok_or_else(|| err("belum ada vault terbuka"))?;
        let path = v.root.join(&rel);
        vault::atomic_write(&path, &content)
            .map_err(|e| e.to_string())
            .map_err(err)?;
    }
    // update indeks + broadcast perubahan ke sesi lain (mis. jendela lain)
    refresh_index(&state, &rel)?;
    let _ = app.emit("vault-changed", serde_json::json!({ "self": true, "paths": [rel] }));
    read_note(state, rel)
}

/// Re-scan satu note ke indeks.
fn refresh_index(state: &State<AppState>, rel: &str) -> Result<(), ErrorJson> {
    let mut guard = state.vault.lock().unwrap();
    let v = guard.as_mut().ok_or_else(|| err("belum ada vault terbuka"))?;
    let path = v.root.join(vault::normalize_rel(rel));
    if path.exists() {
        let raw = std::fs::read_to_string(&path).map_err(|e| e.to_string()).map_err(err)?;
        let note = vault::parse_note(rel, &raw);
        v.index.upsert(note);
    } else {
        v.index.remove(rel);
    }
    Ok(())
}

#[tauri::command]
fn create_note(
    state: State<AppState>,
    rel: String,
    content: Option<String>,
) -> Result<NotePayload, ErrorJson> {
    let rel = vault::ensure_note_path(&rel).map_err(err)?;
    {
        let guard = state.vault.lock().unwrap();
        let v = guard.as_ref().ok_or_else(|| err("belum ada vault terbuka"))?;
        let path = v.root.join(&rel);
        if path.exists() {
            return Err(err(format!("note sudah ada: {rel}")));
        }
        let body = content.unwrap_or_default();
        vault::atomic_write(&path, &body)
            .map_err(|e| e.to_string())
            .map_err(err)?;
    }
    refresh_index(&state, &rel)?;
    read_note(state, rel)
}

#[tauri::command]
fn create_folder(state: State<AppState>, rel: String) -> Result<(), ErrorJson> {
    with_vault(&state, |v| {
        let rel = vault::normalize_rel(&rel);
        if rel.split('/').any(|s| s.starts_with('.')) {
            return Err("nama folder tidak valid".into());
        }
        std::fs::create_dir_all(v.root.join(rel)).map_err(|e| e.to_string())
    })
}

#[tauri::command]
fn rename_path(state: State<AppState>, from: String, to: String) -> Result<(), ErrorJson> {
    let from_n = vault::normalize_rel(&from);
    let to_n = if from_n.to_ascii_lowercase().ends_with(".md") {
        vault::ensure_note_path(&to).map_err(err)?
    } else {
        vault::normalize_rel(&to)
    };
    {
        let guard = state.vault.lock().unwrap();
        let v = guard.as_ref().ok_or_else(|| err("belum ada vault terbuka"))?;
        let src = v.root.join(&from_n);
        let dst = v.root.join(&to_n);
        if !src.exists() {
            return Err(err(format!("tidak ada: {from_n}")));
        }
        if dst.exists() {
            return Err(err(format!("tujuan sudah ada: {to_n}")));
        }
        if let Some(d) = dst.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string()).map_err(err)?;
        }

        // Peta path lama → baru untuk SEMUA note terdampak:
        // - rename file  → satu entri
        // - rename folder → semua note di bawahnya
        let mut map = oxidian_core::rename::RenameMap::new();
        for p in v.index.paths() {
            let moved = if from_n.to_ascii_lowercase().ends_with(".md") {
                p.eq_ignore_ascii_case(&from_n)
            } else {
                let prefix = format!("{}/", from_n);
                p.to_ascii_lowercase().starts_with(&prefix.to_ascii_lowercase())
            };
            if moved {
                let rest = if from_n.to_ascii_lowercase().ends_with(".md") {
                    to_n.clone()
                } else {
                    let rel_inside = p[from_n.len() + 1..].to_string();
                    format!("{to_n}/{rel_inside}")
                };
                map.insert(p, rest);
            }
        }

        std::fs::rename(&src, &dst).map_err(|e| e.to_string()).map_err(err)?;

        // Update semua link di note lain yang menaut ke path lama (ala Obsidian).
        // Dilakukan PAKAI index lama (state belum di-rescan) — sudah dibangun di atas.
        let plans = oxidian_core::rename::plan_link_rewrites(&v.index, &map);
        oxidian_core::rename::apply_rewrites(&v.root, &plans)
            .map_err(|e| e.to_string())
            .map_err(err)?;
    }
    // re-scan: file yang pindah + link yang baru saja ditulis ulang
    rescan(state)?;
    Ok(())
}

#[tauri::command]
fn delete_path(state: State<AppState>, rel: String) -> Result<(), ErrorJson> {
    let rel = vault::normalize_rel(&rel);
    {
        let guard = state.vault.lock().unwrap();
        let v = guard.as_ref().ok_or_else(|| err("belum ada vault terbuka"))?;
        vault::move_to_trash(&v.root, &rel).map_err(err)?;
    }
    rescan(state)?;
    Ok(())
}

/// Full rescan vault (dipakai setelah rename/delete/kejadian watcher).
#[tauri::command]
fn rescan(state: State<AppState>) -> Result<(), ErrorJson> {
    let mut guard = state.vault.lock().unwrap();
    let v = guard.as_mut().ok_or_else(|| err("belum ada vault terbuka"))?;
    let notes = vault::scan_vault(&v.root).map_err(err)?;
    v.index = VaultIndex::build(notes);
    Ok(())
}

#[tauri::command]
fn search_notes(state: State<AppState>, query: String) -> Result<Vec<search::SearchHit>, ErrorJson> {
    with_vault(&state, |v| Ok(search::search(v.index.iter_notes(), &query)))
}

#[tauri::command]
fn backlinks(state: State<AppState>, rel: String) -> Result<Vec<BacklinkItem>, ErrorJson> {
    with_vault(&state, |v| {
        let rel = vault::normalize_rel(&rel);
        let mut out = Vec::new();
        for src in v.index.backlinks_of(&rel) {
            // snippet: baris persis yang memuat link menuju `rel` (via span)
            let snippet = v
                .index
                .get(&src)
                .and_then(|n| {
                    n.links
                        .iter()
                        .find(|l| {
                            matches!(
                                v.index.resolve(&n.rel_path, &l.target),
                                oxidian_core::index::Resolved::Note(ref p) if *p == rel
                            )
                        })
                        .and_then(|l| {
                            if l.start > n.body.len() || l.end > n.body.len() {
                                return None;
                            }
                            let line_start = n.body[..l.start].rfind('\n').map(|i| i + 1).unwrap_or(0);
                            let line_end = n.body[l.end..]
                                .find('\n')
                                .map(|i| l.end + i)
                                .unwrap_or(n.body.len());
                            Some(
                                n.body[line_start..line_end]
                                    .trim()
                                    .chars()
                                    .take(140)
                                    .collect::<String>(),
                            )
                        })
                })
                .unwrap_or_default();
            let note = v.index.get(&src);
            out.push(BacklinkItem {
                rel_path: src.clone(),
                title: note.map(|n| n.title.clone()).unwrap_or_else(|| vault::title_of(&src)),
                snippet,
            });
        }
        Ok(out)
    })
}

#[tauri::command]
fn all_tags(state: State<AppState>) -> Result<Vec<(String, usize)>, ErrorJson> {
    with_vault(&state, |v| {
        Ok(v.index
            .tags()
            .iter()
            .map(|(t, notes)| (t.clone(), notes.len()))
            .collect())
    })
}

#[tauri::command]
fn notes_with_tag(state: State<AppState>, tag: String) -> Result<Vec<String>, ErrorJson> {
    with_vault(&state, |v| {
        Ok(v.index
            .tags()
            .get(&tag.to_lowercase())
            .cloned()
            .unwrap_or_default())
    })
}

#[tauri::command]
fn graph_data(state: State<AppState>) -> Result<GraphPayload, ErrorJson> {
    with_vault(&state, |v| {
        let (nodes, edges) = v.index.graph();
        Ok(GraphPayload { nodes, edges })
    })
}

/// Resolve klik wikilink dari note sumber.
#[tauri::command]
fn resolve_link(state: State<AppState>, from: String, target: String) -> Result<Option<String>, ErrorJson> {
    with_vault(&state, |v| {
        Ok(match v.index.resolve(&vault::normalize_rel(&from), &target) {
            oxidian_core::index::Resolved::Note(p) => Some(p),
            oxidian_core::index::Resolved::Unresolved(_) => None,
        })
    })
}

#[tauri::command]
fn set_last_note(state: State<AppState>, rel: Option<String>) -> Result<(), ErrorJson> {
    let mut cfg = state.config.lock().unwrap();
    cfg.last_note = rel;
    cfg.save().map_err(err)?;
    Ok(())
}

/// Simpan tema pilihan user (light/dark) ke config.
#[tauri::command]
fn set_theme(state: State<AppState>, theme: String) -> Result<(), ErrorJson> {
    let mut cfg = state.config.lock().unwrap();
    cfg.theme = if theme == "light" { "light".into() } else { "dark".into() };
    cfg.save().map_err(err)?;
    Ok(())
}

/// Buka Graph View di window TERPISAH (bukan overlay).
/// Bila sudah terbuka, window yang ada difokuskan.
#[tauri::command]
fn open_graph_window(app: AppHandle) -> Result<(), ErrorJson> {
    if let Some(existing) = app.get_webview_window("graph") {
        let _ = existing.set_focus();
        return Ok(());
    }
    WebviewWindowBuilder::new(&app, "graph", WebviewUrl::App("graph.html".into()))
        .title("Oxidian — Graph View")
        .inner_size(960.0, 680.0)
        .min_inner_size(420.0, 320.0)
        .build()
        .map_err(|e| e.to_string())
        .map_err(err)?;
    Ok(())
}

#[tauri::command]
fn all_notes(state: State<AppState>) -> Result<Vec<(String, String)>, ErrorJson> {
    with_vault(&state, |v| Ok(v.index.all_titles()))
}

/// Rescan ringan ketika watcher memberi tahu perubahan eksternal.
#[tauri::command]
fn apply_external_changes(state: State<AppState>, rels: Vec<String>) -> Result<(), ErrorJson> {
    for rel in rels {
        let _ = refresh_index(&state, &rel);
    }
    Ok(())
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let config = Config::load();
            let shared = Arc::new(AppStateInner {
                vault: Mutex::new(None),
                config: Mutex::new(config.clone()),
            });
            // MCP server untuk akses agent AI — bind localhost saja
            if let Some(port) = config.mcp_port {
                mcp::spawn_server(shared.clone(), port);
            }
            app.manage(shared);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            open_vault,
            create_vault,
            tree,
            read_note,
            write_note,
            create_note,
            create_folder,
            rename_path,
            delete_path,
            rescan,
            search_notes,
            backlinks,
            all_tags,
            notes_with_tag,
            graph_data,
            resolve_link,
            set_last_note,
            set_theme,
            open_graph_window,
            all_notes,
            apply_external_changes
        ])
        .run(tauri::generate_context!())
        .expect("gagal menjalankan Oxidian");
}
