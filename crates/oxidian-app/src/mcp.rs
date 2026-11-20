//! MCP (Model Context Protocol) server — HTTP JSON-RPC di `127.0.0.1:<port>/mcp`.
//!
//! AI agent (Claude, ZCode, dsb.) bisa terhubung dan mengakses vault yang
//! sedang terbuka: list/read/write/create/delete note, search, backlinks,
//! tags, dan ringkasan graph. Server **stateless** (tanpa session), hanya
//! listen di localhost, dan berbagi state yang sama dengan GUI.

use crate::{vault as app_vault, AppState, ActiveVault};
use oxidian_core::{search, vault};
use serde_json::{json, Value};

/// Jalankan server MCP di thread background ( tidak pernah memblok GUI ).
pub fn spawn_server(state: AppState, port: u16) {
    std::thread::spawn(move || run_server(state, port));
}

fn run_server(state: AppState, port: u16) {
    let server = match tiny_http::Server::http(("127.0.0.1", port)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[mcp] gagal bind 127.0.0.1:{port}: {e}");
            return;
        }
    };
    eprintln!("[mcp] berjalan di http://127.0.0.1:{port}/mcp");

    for mut request in server.incoming_requests() {
        let method = request.method().clone();
        let url = request.url().to_string();
        let mut body = String::new();
        let _ = request.as_reader().read_to_string(&mut body);

        let (code, payload, content_type): (u16, String, String) = match (&method, url.as_str()) {
            // CORS preflight
            (tiny_http::Method::Options, _) => {
                (200, String::new(), "text/plain".to_string())
            }
            (tiny_http::Method::Get, u) if u.starts_with("/mcp") => (
                405,
                json!({"error": "gunakan POST untuk JSON-RPC (streamable HTTP)"})
                    .to_string(),
                "application/json".to_string(),
            ),
            (tiny_http::Method::Post, u) if u.starts_with("/mcp") => {
                let (code, payload) = handle_post(&state, &body);
                (code, payload, "application/json".to_string())
            }
            _ => (404, json!({"error": "not found"}).to_string(), "application/json".to_string()),
        };

        let mut response = tiny_http::Response::from_string(payload).with_status_code(code);
        // notification → 202 tanpa isi/body header tambahan tak penting
        let header_list: Vec<(&str, &str)> = if code == 202 {
            vec![]
        } else {
            vec![
                ("Content-Type", content_type.as_str()),
                ("Access-Control-Allow-Origin", "*"),
                ("Access-Control-Allow-Headers", "Content-Type, Mcp-Session-Id, Authorization"),
                ("Access-Control-Allow-Methods", "POST, GET, OPTIONS"),
            ]
        };
        for (k, v) in header_list {
            if let Ok(h) = tiny_http::Header::from_bytes(k.as_bytes(), v.as_bytes()) {
                response.add_header(h);
            }
        }
        let _ = request.respond(response);
    }
}

/// Tangani satu POST JSON-RPC.
fn handle_post(state: &AppState, body: &str) -> (u16, String) {
    let parsed: Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return (400, rpc_error(Value::Null, -32700, "Parse error")),
    };
    let id = parsed.get("id").cloned().unwrap_or(Value::Null);
    let method = parsed
        .get("method")
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string();

    // notification: tidak butuh response
    if method.starts_with("notifications/") {
        return (202, String::new());
    }
    let params = parsed.get("params").cloned().unwrap_or(json!({}));

    let result = match method.as_str() {
        "initialize" => Ok(json!({
            "protocolVersion": "2025-03-26",
            "capabilities": { "tools": {}, "resources": {} },
            "serverInfo": { "name": "oxidian", "version": env!("CARGO_PKG_VERSION") }
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools_list() })),
        "tools/call" => {
            let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            match run_tool(state, &name, &args) {
                Ok(text) => Ok(json!({ "content": [{ "type": "text", "text": text }] })),
                Err(msg) => Ok(json!({
                    "content": [{ "type": "text", "text": msg }],
                    "isError": true
                })),
            }
        }
        "resources/list" => Ok(json!({ "resources": [] })),
        "resources/templates/list" => Ok(json!({ "resourceTemplates": [] })),
        "prompts/list" => Ok(json!({ "prompts": [] })),
        _ => Err((-32601, format!("Method not found: {method}"))),
    };

    match result {
        Ok(r) => (200, json!({ "jsonrpc": "2.0", "id": id, "result": r }).to_string()),
        Err((code, msg)) => (200, rpc_error(id, code, &msg)),
    }
}

fn rpc_error(id: Value, code: i32, message: &str) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
        .to_string()
}

// ============================================================
// TOOLS
// ============================================================

fn tools_list() -> Value {
    json!([
    {
        "name": "vault_info",
        "description": "Info vault Oxidian yang sedang terbuka: path, jumlah note, jumlah tag.",
        "inputSchema": { "type": "object", "properties": {} }
    },
    {
        "name": "list_notes",
        "description": "Daftar semua note di vault (judul + path relatif).",
        "inputSchema": { "type": "object", "properties": {} }
    },
    {
        "name": "read_note",
        "description": "Baca isi lengkap satu note (markdown mentah, termasuk frontmatter).",
        "inputSchema": {
            "type": "object",
            "properties": { "rel": { "type": "string", "description": "Path relatif note, mis. 'Rust/Ownership.md'" } },
            "required": ["rel"]
        }
    },
    {
        "name": "write_note",
        "description": "Buat atau timpa note (upsert). Menulis atomik; file .md ditambahkan otomatis bila belum ada.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "rel": { "type": "string", "description": "Path relatif note" },
                "content": { "type": "string", "description": "Isi note dalam markdown" }
            },
            "required": ["rel", "content"]
        }
    },
    {
        "name": "create_note",
        "description": "Buat note BARU. Gagal bila note sudah ada (gunakan write_note untuk menimpa).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "rel": { "type": "string" },
                "content": { "type": "string", "description": "Opsional; default heading dari nama file" }
            },
            "required": ["rel"]
        }
    },
    {
        "name": "delete_note",
        "description": "Hapus note — dipindah ke folder .trash/ di dalam vault (soft delete).",
        "inputSchema": {
            "type": "object",
            "properties": { "rel": { "type": "string" } },
            "required": ["rel"]
        }
    },
    {
        "name": "search",
        "description": "Cari di seluruh vault: judul, tag, dan konten. Return daftar hit dengan skor & snippet.",
        "inputSchema": {
            "type": "object",
            "properties": { "query": { "type": "string" } },
            "required": ["query"]
        }
    },
    {
        "name": "backlinks",
        "description": "Daftar note yang menaut ke note tertentu (dengan baris konteks).",
        "inputSchema": {
            "type": "object",
            "properties": { "rel": { "type": "string" } },
            "required": ["rel"]
        }
    },
    {
        "name": "list_tags",
        "description": "Daftar semua tag di vault beserta jumlah note-nya.",
        "inputSchema": { "type": "object", "properties": {} }
    },
    {
        "name": "notes_with_tag",
        "description": "Daftar note yang memuat tag tertentu.",
        "inputSchema": {
            "type": "object",
            "properties": { "tag": { "type": "string" } },
            "required": ["tag"]
        }
    },
    {
        "name": "graph_summary",
        "description": "Ringkasan graph pengetahuan: jumlah node/edge dan derajat tiap note.",
        "inputSchema": { "type": "object", "properties": {} }
    }
    ])
}

fn run_tool(state: &AppState, name: &str, args: &Value) -> Result<String, String> {
    let rel_arg = || -> Result<String, String> {
        Ok(args
            .get("rel")
            .and_then(|v| v.as_str())
            .ok_or("argument 'rel' wajib")?
            .to_string())
    };
    match name {
        "vault_info" => {
            let guard = state.vault.lock().map_err(|_| "lock poisoned")?;
            match guard.as_ref() {
                Some(v) => Ok(format!(
                    "vault: {}\nnote: {}",
                    v.root.display(),
                    v.index.len()
                )),
                None => Ok("belum ada vault terbuka di aplikasi Oxidian".into()),
            }
        }
        "list_notes" => {
            let guard = state.vault.lock().map_err(|_| "lock poisoned")?;
            let v = guard.as_ref().ok_or("belum ada vault terbuka")?;
            let mut out = String::new();
            for (rel, title) in v.index.all_titles() {
                out.push_str(&format!("- {title} — {rel}\n"));
            }
            Ok(out)
        }
        "read_note" => {
            let rel = rel_arg()?;
            let guard = state.vault.lock().map_err(|_| "lock poisoned")?;
            let v = guard.as_ref().ok_or("belum ada vault terbuka")?;
            let rel = vault::normalize_rel(&rel);
            let raw = std::fs::read_to_string(v.root.join(&rel))
                .map_err(|e| format!("{rel} tidak bisa dibaca: {e}"))?;
            Ok(raw)
        }
        "write_note" => {
            let rel = rel_arg()?;
            let content = args
                .get("content")
                .and_then(|v| v.as_str())
                .ok_or("argument 'content' wajib")?;
            let rel = vault::ensure_note_path(&rel).map_err(|e| e.to_string())?;
            {
                let mut guard = state.vault.lock().map_err(|_| "lock poisoned")?;
                let v = guard.as_mut().ok_or("belum ada vault terbuka")?;
                app_vault::atomic_write(&v.root.join(&rel), content)
                    .map_err(|e| format!("gagal menulis: {e}"))?;
                let raw = std::fs::read_to_string(v.root.join(&rel))
                    .map_err(|e| e.to_string())?;
                v.index.upsert(vault::parse_note(&rel, &raw));
            }
            Ok(format!("✓ note ditulis: {rel}"))
        }
        "create_note" => {
            let rel = rel_arg()?;
            let rel = vault::ensure_note_path(&rel).map_err(|e| e.to_string())?;
            let content = args
                .get("content")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("# {}\n\n", vault::title_of(&rel)));
            {
                let mut guard = state.vault.lock().map_err(|_| "lock poisoned")?;
                let v = guard.as_mut().ok_or("belum ada vault terbuka")?;
                let path = v.root.join(&rel);
                if path.exists() {
                    return Err(format!("note sudah ada: {rel} (pakai write_note untuk menimpa)"));
                }
                app_vault::atomic_write(&path, &content).map_err(|e| format!("gagal menulis: {e}"))?;
                let raw = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
                v.index.upsert(vault::parse_note(&rel, &raw));
            }
            Ok(format!("✓ note dibuat: {rel}"))
        }
        "delete_note" => {
            let rel = rel_arg()?;
            let rel = vault::normalize_rel(&rel);
            {
                let mut guard = state.vault.lock().map_err(|_| "lock poisoned")?;
                let v = guard.as_mut().ok_or("belum ada vault terbuka")?;
                app_vault::move_to_trash(&v.root, &rel).map_err(|e| e.to_string())?;
                v.index.remove(&rel);
            }
            Ok(format!("✓ note dipindah ke .trash: {rel}"))
        }
        "search" => {
            let query = args
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or("argument 'query' wajib")?;
            let guard = state.vault.lock().map_err(|_| "lock poisoned")?;
            let v = guard.as_ref().ok_or("belum ada vault terbuka")?;
            let hits = search::search(v.index.iter_notes(), query);
            if hits.is_empty() {
                return Ok(format!("tidak ada hasil untuk \"{query}\""));
            }
            let mut out = String::new();
            for (i, h) in hits.iter().take(20).enumerate() {
                out.push_str(&format!(
                    "{}. {} — {} (skor {})\n   {}\n",
                    i + 1,
                    h.title,
                    h.rel_path,
                    h.score,
                    h.snippet
                ));
            }
            Ok(out)
        }
        "backlinks" => {
            let rel = rel_arg()?;
            let rel = vault::normalize_rel(&rel);
            let guard = state.vault.lock().map_err(|_| "lock poisoned")?;
            let v = guard.as_ref().ok_or("belum ada vault terbuka")?;
            let sources = v.index.backlinks_of(&rel);
            if sources.is_empty() {
                return Ok(format!("tidak ada backlink menuju {rel}"));
            }
            let mut out = String::new();
            for src in sources {
                let title = v
                    .index
                    .get(&src)
                    .map(|n| n.title.clone())
                    .unwrap_or_else(|| vault::title_of(&src));
                out.push_str(&format!("- {title} — {src}\n"));
            }
            Ok(out)
        }
        "list_tags" => {
            let guard = state.vault.lock().map_err(|_| "lock poisoned")?;
            let v = guard.as_ref().ok_or("belum ada vault terbuka")?;
            let mut out = String::new();
            for (tag, notes) in v.index.tags() {
                out.push_str(&format!("- #{tag} ({})\n", notes.len()));
            }
            Ok(out)
        }
        "notes_with_tag" => {
            let tag = args
                .get("tag")
                .and_then(|v| v.as_str())
                .ok_or("argument 'tag' wajib")?;
            let guard = state.vault.lock().map_err(|_| "lock poisoned")?;
            let v = guard.as_ref().ok_or("belum ada vault terbuka")?;
            let notes = v
                .index
                .tags()
                .get(&tag.trim_start_matches('#').to_lowercase())
                .cloned()
                .unwrap_or_default();
            if notes.is_empty() {
                return Ok(format!("tidak ada note dengan tag #{tag}"));
            }
            Ok(notes
                .iter()
                .map(|n| {
                    let t = v
                        .index
                        .get(n)
                        .map(|x| x.title.clone())
                        .unwrap_or_else(|| vault::title_of(n));
                    format!("- {t} — {n}")
                })
                .collect::<Vec<_>>()
                .join("\n"))
        }
        "graph_summary" => {
            let guard = state.vault.lock().map_err(|_| "lock poisoned")?;
            let v = guard.as_ref().ok_or("belum ada vault terbuka")?;
            let (nodes, edges) = v.index.graph();
            let mut ranked: Vec<&oxidian_core::index::GraphNode> =
                nodes.iter().collect();
            ranked.sort_by(|a, b| b.link_count.cmp(&a.link_count));
            let mut out = format!(
                "graph: {} node, {} link ter-resolve\n\ntop terhubung:\n",
                nodes.len(),
                edges.len()
            );
            for n in ranked.iter().take(10) {
                out.push_str(&format!("- {} ({} link) — {}\n", n.title, n.link_count, n.id));
            }
            Ok(out)
        }
        _ => Err(format!("tool tidak dikenal: {name}")),
    }
}

// jaga agar ActiveVault tetap dianggap dipakai di modul ini (tipe dipakai lewat AppState)
#[allow(dead_code)]
fn _assert_types(_v: &ActiveVault) {}
