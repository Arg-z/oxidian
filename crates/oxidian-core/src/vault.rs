//! Scan vault: folder berisi file `.md` (format vault Obsidian).
//! Folder `.obsidian`, `.trash`, `.git`, dan file tersembunyi diabaikan —
//! tidak pernah dibaca maupun ditulis.

use crate::links;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Error domain core.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("vault tidak ditemukan: {0}")]
    VaultNotFound(String),
    #[error("path di luar vault: {0}")]
    PathOutsideVault(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}
pub type Result<T> = std::result::Result<T, CoreError>;

/// Satu note hasil scan: metadata + hasil parsing konten.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteData {
    /// Path relatif terhadap root vault, selalu dengan separator `/`. Contoh: `Rust/Notes.md`.
    pub rel_path: String,
    /// Judul = nama file tanpa `.md` (konvensi Obsidian).
    pub title: String,
    /// Frontmatter YAML sebagai JSON object (kosong jika tidak ada).
    pub frontmatter: BTreeMap<String, serde_json::Value>,
    /// Tag gabungan frontmatter + inline.
    pub tags: Vec<String>,
    /// Semua wikilink mentah.
    pub links: Vec<links::RawLink>,
    /// Konten Markdown **tanpa** blok frontmatter.
    pub body: String,
    /// Konten penuh termasuk frontmatter (untuk editor).
    #[serde(skip)]
    pub raw: String,
}

/// Nama folder di dalam vault yang diabaikan sepenuhnya.
pub const IGNORED_DIRS: &[&str] = &[".obsidian", ".trash", ".git", ".oxidian"];

/// Normalisasi path relatif → selalu `/`, tanpa `./`, tanpa komponen kosong.
pub fn normalize_rel(p: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for c in p.split(['/', '\\']) {
        match c {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

/// Judul note dari nama file (tanpa ekstensi).
pub fn title_of(rel_path: &str) -> String {
    let base = rel_path.rsplit('/').next().unwrap_or(rel_path);
    base.strip_suffix(".md").unwrap_or(base).to_string()
}

/// Validasi: rel aman & diakhiri .md.
pub fn ensure_note_path(rel: &str) -> Result<String> {
    let rel = normalize_rel(rel);
    if rel.is_empty() {
        return Err(CoreError::PathOutsideVault(rel));
    }
    for seg in rel.split('/') {
        if seg.starts_with('.') {
            return Err(CoreError::PathOutsideVault(seg.to_string()));
        }
    }
    let rel = if rel.ends_with(".md") {
        rel
    } else {
        format!("{rel}.md")
    };
    Ok(rel)
}

/// Parse satu file `.md` mentah menjadi [`NoteData`].
pub fn parse_note(rel_path: &str, raw: &str) -> NoteData {
    // gray_matter 0.3: parse generik → langsung ke serde_json::Value.
    // YAML rusak → frontmatter kosong, body = raw (graceful).
    let (frontmatter, body) = match gray_matter::Matter::<gray_matter::engine::YAML>::new()
        .parse::<serde_json::Value>(raw)
    {
        Ok(p) => (
            match p.data {
                Some(serde_json::Value::Object(map)) => map.into_iter().collect(),
                _ => BTreeMap::new(),
            },
            p.content,
        ),
        Err(_) => (BTreeMap::new(), raw.to_string()),
    };
    let inline_tags = links::parse_tags(&body);
    let tags = links::merge_frontmatter_tags(frontmatter.get("tags"), &inline_tags);
    NoteData {
        rel_path: normalize_rel(rel_path),
        title: title_of(rel_path),
        frontmatter,
        tags,
        links: links::parse_links(&body),
        raw: raw.to_string(),
        body,
    }
}

/// Scan seluruh vault di `root` → daftar note terurut oleh path.
pub fn scan_vault(root: &Path) -> Result<Vec<NoteData>> {
    if !root.is_dir() {
        return Err(CoreError::VaultNotFound(root.display().to_string()));
    }
    let mut notes = Vec::new();
    for entry in WalkDir::new(root)
        .min_depth(1)
        .sort_by_file_name()
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.file_type().is_dir() {
            continue; // isi folder tersembunyi disaring di cek komponen path di bawah
        }
        let rel = match entry.path().strip_prefix(root) {
            Ok(r) => r.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };
        // skip semua komponen tersembunyi / ignored
        if rel
            .split('/')
            .any(|seg| seg.starts_with('.') || IGNORED_DIRS.contains(&seg))
        {
            continue;
        }
        if !rel.to_ascii_lowercase().ends_with(".md") {
            continue;
        }
        let raw = std::fs::read_to_string(entry.path())?;
        notes.push(parse_note(&rel, &raw));
    }
    notes.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    Ok(notes)
}

/// Tulis file secara atomik (tmp di folder sama + rename) supaya tidak pernah
/// menyisakan file setengah tertulis — vault tetap konsisten.
pub fn atomic_write(path: &Path, content: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("md.oxtmp");
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, path)
}

/// Pindahkan ke `.trash/` di dalam vault (soft-delete gaya Obsidian).
pub fn move_to_trash(root: &Path, rel: &str) -> Result<PathBuf> {
    let rel = normalize_rel(rel);
    let src = root.join(&rel);
    if !src.exists() {
        return Err(CoreError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("{rel} tidak ada"),
        )));
    }
    let trash = root.join(".trash");
    std::fs::create_dir_all(&trash)?;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let name = format!("{ts}-{}", rel.replace('/', "_"));
    let dst = trash.join(name);
    std::fs::rename(&src, &dst)?;
    Ok(dst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_note_dengan_frontmatter() {
        let raw = "---\ntitle: Halo\ntags: [a, b]\n---\n\nIsi #inline dan [[Link Saya]]\n";
        let n = parse_note("folder/Halo.md", raw);
        assert_eq!(n.title, "Halo");
        assert_eq!(n.frontmatter.get("title").unwrap(), "Halo");
        assert_eq!(n.tags, vec!["a", "b", "inline"]);
        assert_eq!(n.links.len(), 1);
        assert_eq!(n.body.trim(), "Isi #inline dan [[Link Saya]]");
    }

    #[test]
    fn parse_note_tanpa_frontmatter() {
        let n = parse_note("x.md", "cuma teks");
        assert!(n.frontmatter.is_empty());
        assert_eq!(n.body, "cuma teks");
    }

    #[test]
    fn parse_note_frontmatter_rusak() {
        // YAML invalid → frontmatter kosong, isi tetap dibaca (graceful)
        let n = parse_note("x.md", "---\n: : :\n---\nisi");
        assert_eq!(n.body, "isi");
    }

    #[test]
    fn scan_vault_mengabaikan_folder_tersembunyi() {
        let tmp = tempdir();
        let root = &tmp;
        std::fs::write(root.join("a.md"), "isi a [[b]]").unwrap();
        std::fs::create_dir_all(root.join(".obsidian")).unwrap();
        std::fs::write(root.join(".obsidian/workspace.json"), "{}").unwrap();
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("sub/b.md"), "#b").unwrap();
        std::fs::write(root.join("sub/c.txt"), "bukan md").unwrap();
        let notes = scan_vault(root).unwrap();
        let paths: Vec<_> = notes.iter().map(|n| n.rel_path.as_str()).collect();
        assert_eq!(paths, vec!["a.md", "sub/b.md"]);
    }

    #[test]
    fn normalize_dan_ensure_path() {
        assert_eq!(normalize_rel("./a//b.md"), "a/b.md");
        assert_eq!(normalize_rel("a/../../x"), "x");
        assert_eq!(ensure_note_path("Folder/Note").unwrap(), "Folder/Note.md");
        assert!(ensure_note_path(".git/config").is_err());
    }

    #[test]
    fn atomic_write_dan_trash() {
        let tmp = tempdir();
        let p = &tmp.join("n.md");
        atomic_write(&p, "v1").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "v1");
        atomic_write(&p, "v2").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "v2");
        let dst = move_to_trash(&tmp, "n.md").unwrap();
        assert!(dst.exists());
        assert!(!p.exists());
    }

    /// tempdir minimal tanpa dependensi tambahan.
    fn tempdir() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "oxidian-test-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }
}
