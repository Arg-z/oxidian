//! Penulisan ulang link di seluruh vault saat note/folder di-rename
//! (perilaku Obsidian: rename tidak boleh merusak backlink).
//!
//! Prasyarat: [`crate::links::strip_code`] mempertahankan panjang string,
//! sehingga span pada [`crate::links::RawLink`] valid untuk direkonstruksi.

use crate::index::{Resolved, VaultIndex};
use crate::vault::atomic_write;
use std::collections::BTreeMap;
use std::path::Path;

/// Peta path lama → path baru (keduanya rel, dengan `.md`).
pub type RenameMap = BTreeMap<String, String>;

/// Satu file yang kontennya perlu ditulis ulang.
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedRewrite {
    /// Path rel (baru, bila note itu sendiri ikut pindah) untuk menulis.
    pub path: String,
    /// Konten baru (frontmatter asli + body dengan link yang diperbarui).
    pub content: String,
    /// Jumlah link yang diganti di file ini.
    pub changed_links: usize,
}

/// Rencanakan penulisan ulang semua note yang menaut ke path lama.
///
/// `index` harus masih berisi **state lama** (sebelum rename di filesystem),
/// dan `map` berisi `old_rel.md → new_rel.md` (untuk rename folder: semua
/// note di bawah folder tersebut).
pub fn plan_link_rewrites(index: &VaultIndex, map: &RenameMap) -> Vec<PlannedRewrite> {
    let mut out = Vec::new();
    for note in index.iter_notes() {
        let mut reps: Vec<(usize, usize, String)> = Vec::new();
        for link in &note.links {
            let old_target = match index.resolve(&note.rel_path, &link.target) {
                Resolved::Note(p) => p,
                Resolved::Unresolved(_) => continue,
            };
            let Some(new_rel) = map.get(&old_target) else {
                continue;
            };
            let Some(new_target_text) = rewrite_target_text(&link.target, new_rel) else {
                continue;
            };
            let mut inner = new_target_text;
            if let Some(h) = &link.heading {
                inner.push('#');
                inner.push_str(h);
            }
            if let Some(a) = &link.alias {
                inner.push('|');
                inner.push_str(a);
            }
            reps.push((link.start, link.end, format!("[[{inner}]]")));
        }
        if reps.is_empty() {
            continue;
        }

        // Bangun body baru dari span (terurut naik, tidak tumpang tindih)
        let mut new_body = String::with_capacity(note.body.len());
        let mut last = 0usize;
        for (s, e, txt) in &reps {
            if *s < last || *e > note.body.len() {
                return out; // span tak valid — jangan korbankan file
            }
            new_body.push_str(&note.body[last..*s]);
            new_body.push_str(txt);
            last = *e;
        }
        new_body.push_str(&note.body[last..]);

        // Rekonstruksi raw: cari posisi body di raw (gray_matter bisa membuang
        // newline kosong setelah `---`, jadi body tak selalu suffix persis).
        // Frontmatter = raw sebelum posisi body — dipertahankan apa adanya.
        let raw = &note.raw;
        if note.body.is_empty() {
            continue;
        }
        let Some(body_pos) = raw.rfind(&note.body) else {
            continue;
        };
        // gray_matter bisa memangkas newline di awal/ujung body —
        // pertahankan apa adanya bagian raw sebelum & sesudah body.
        let tail = &raw[body_pos + note.body.len()..];
        if !tail.chars().all(|c| c.is_whitespace()) {
            continue; // fallback aman: file di-skip, tidak pernah dirusak
        }
        let new_content = format!("{}{}{}", &raw[..body_pos], new_body, tail);
        let path = map
            .get(&note.rel_path)
            .cloned()
            .unwrap_or_else(|| note.rel_path.clone());
        out.push(PlannedRewrite {
            path,
            content: new_content,
            changed_links: reps.len(),
        });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Teks target baru untuk sebuah link yang kini menunjuk `new_rel`:
/// - path-style (target mengandung `/`) → path baru tanpa `.md`
/// - basename-style → basename baru (folder asal tetap)
/// - `None` bila teks tidak berubah (bukan wajib ditulis ulang)
fn rewrite_target_text(target: &str, new_rel: &str) -> Option<String> {
    let new_noext = new_rel.strip_suffix(".md").unwrap_or(new_rel);
    if target.contains('/') {
        if target.trim_end_matches(".md") == new_noext {
            return None; // sudah sama
        }
        Some(new_noext.to_string())
    } else {
        let base = new_noext.rsplit('/').next().unwrap_or(new_noext);
        if base == target {
            return None; // basename tidak berubah (mis. rename folder)
        }
        Some(base.to_string())
    }
}

/// Tulis semua rencana ke disk (atomik). Return total link yang berubah.
pub fn apply_rewrites(root: &Path, plans: &[PlannedRewrite]) -> std::io::Result<usize> {
    let mut total = 0usize;
    for p in plans {
        atomic_write(&root.join(&p.path), &p.content)?;
        total += p.changed_links;
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::{parse_note, scan_vault};

    fn index_from(pairs: &[(&str, &str)]) -> VaultIndex {
        VaultIndex::build(pairs.iter().map(|(p, r)| parse_note(p, r)).collect())
    }

    #[test]
    fn rename_file_update_link_path_dan_basename() {
        let idx = index_from(&[
            (
                "a.md",
                "---\ntitle: A\n---\n\nke [[Rust/Lama|lama]] dan [[Lama#Catatan]] juga [[Lain]]\n",
            ),
            ("Rust/Lama.md", "isi"),
            ("Lain.md", "isi"),
        ]);
        let mut map = RenameMap::new();
        map.insert("Rust/Lama.md".into(), "Rust/Baru.md".into());
        let plans = plan_link_rewrites(&idx, &map);
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].path, "a.md");
        assert_eq!(plans[0].changed_links, 2);
        assert_eq!(
            plans[0].content,
            "---\ntitle: A\n---\n\nke [[Rust/Baru|lama]] dan [[Baru#Catatan]] juga [[Lain]]\n"
        );
    }

    #[test]
    fn rename_folder_update_prefix_path_saja() {
        let idx = index_from(&[
            ("x.md", "lihat [[Rust/Notes]] dan [[Notes]]"),
            ("Rust/Notes.md", "isi"),
            ("Java/Notes.md", "isi"),
        ]);
        let mut map = RenameMap::new();
        map.insert("Rust/Notes.md".into(), "RS/Notes.md".into());
        let plans = plan_link_rewrites(&idx, &map);
        assert_eq!(plans.len(), 1);
        // path-style ikut berubah; basename "Notes" tetap (masih resolve)
        assert_eq!(plans[0].content, "lihat [[RS/Notes]] dan [[Notes]]");
    }

    #[test]
    fn rename_note_yang_ditaut_banyak_note() {
        let idx = index_from(&[
            ("a.md", "ke [[B]]"),
            ("b.md", "juga ke [[B|be]]"),
            ("B.md", "isi B"),
        ]);
        let mut map = RenameMap::new();
        map.insert("B.md".into(), "C.md".into());
        let plans = plan_link_rewrites(&idx, &map);
        assert_eq!(plans.len(), 2);
        assert_eq!(
            plans.iter().find(|p| p.path == "a.md").unwrap().content,
            "ke [[C]]"
        );
        assert_eq!(
            plans.iter().find(|p| p.path == "b.md").unwrap().content,
            "juga ke [[C|be]]"
        );
    }

    #[test]
    fn end_to_end_scan_rename_scan() {
        let tmp = std::env::temp_dir().join(format!("oxi-rename-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("a.md"), "ke [[Lama]] ya\n").unwrap();
        std::fs::write(tmp.join("Lama.md"), "isi\n").unwrap();

        let idx = VaultIndex::build(scan_vault(&tmp).unwrap());
        std::fs::rename(tmp.join("Lama.md"), tmp.join("Baru.md")).unwrap();

        let mut map = RenameMap::new();
        map.insert("Lama.md".into(), "Baru.md".into());
        let plans = plan_link_rewrites(&idx, &map);
        assert_eq!(apply_rewrites(&tmp, &plans).unwrap(), 1);

        let idx2 = VaultIndex::build(scan_vault(&tmp).unwrap());
        assert_eq!(
            idx2.resolve("a.md", "Lama"),
            Resolved::Unresolved("Lama".into())
        );
        assert_eq!(
            idx2.resolve("a.md", "Baru"),
            Resolved::Note("Baru.md".into())
        );
        // isi file di disk dipertahankan byte-per-byte (kecuali link yang diganti)
        assert_eq!(
            std::fs::read_to_string(tmp.join("a.md")).unwrap(),
            "ke [[Baru]] ya\n"
        );
        std::fs::remove_dir_all(&tmp).ok();
    }
}
