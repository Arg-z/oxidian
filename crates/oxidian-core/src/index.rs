//! [`VaultIndex`]: indeks in-memory seluruh vault.
//! Resolusi link mengikuti aturan Obsidian yang terdokumentasi:
//! 1. cocok path relatif penuh tanpa `.md` (`Rust/Notes`)
//! 2. cocok basename note (`Notes`) — jika ganda, prioritaskan satu folder dengan sumber
//! 3. case-insensitive
//!
//! Backlink = semua note yang menaut ke note tersebut (resolve dua arah).

use crate::vault::NoteData;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Edge graph: dari note sumber ke note target (sudah resolve).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub source: String,
    pub target: String,
}
// Edge: Ord dibutuhkan untuk edges.sort()
impl PartialEq for Edge {
    fn eq(&self, o: &Self) -> bool {
        self.source == o.source && self.target == o.target
    }
}
impl Eq for Edge {}
impl PartialOrd for Edge {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Edge {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        (self.source.as_str(), self.target.as_str()).cmp(&(o.source.as_str(), o.target.as_str()))
    }
}

/// Node graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: String,      // rel_path
    pub title: String,
    pub link_count: usize, // degree = jumlah edge
}

/// Hasil resolve sebuah link.
#[derive(Debug, Clone, PartialEq)]
pub enum Resolved {
    /// Link mengarah ke note yang ada.
    Note(String),
    /// Target tidak ditemukan di vault (link merah di Obsidian).
    Unresolved(String),
}

#[derive(Debug, Clone, Default)]
pub struct VaultIndex {
    /// rel_path (exact) → note
    notes: HashMap<String, NoteData>,
    /// lowercase rel_path tanpa .md → rel_path (untuk resolusi path penuh)
    by_lpath: HashMap<String, String>,
    /// lowercase basename tanpa .md → daftar rel_path (untuk resolusi basename)
    by_title: HashMap<String, Vec<String>>,
    /// rel_path → rel_path yang menaut ke dia (backlinks)
    backlinks: HashMap<String, HashSet<String>>,
    /// tag (lowercase) → daftar rel_path
    tag_map: BTreeMap<String, Vec<String>>,
}

impl VaultIndex {
    /// Bangun indeks dari hasil scan.
    /// Backlink dihitung di pass kedua supaya arah link ke note yang di-scan
    /// belakangan tetap terdeteksi (perilaku sama dengan Obsidian).
    pub fn build(notes: Vec<NoteData>) -> Self {
        let mut idx = VaultIndex {
            notes: HashMap::new(),
            by_lpath: HashMap::new(),
            by_title: HashMap::new(),
            backlinks: HashMap::new(),
            tag_map: BTreeMap::new(),
        };
        for n in notes {
            idx.insert_raw(n);
        }
        idx.relink_all();
        idx
    }

    /// Reindeks satu note (setelah create/edit). Peta backlink dihitung ulang
    /// penuh supaya link menuju note ini dari note lain ikut konsisten.
    pub fn upsert(&mut self, note: NoteData) -> &mut Self {
        self.remove(&note.rel_path);
        self.insert_raw(note);
        self.relink_all();
        self
    }

    /// Hitung ulang seluruh peta backlink dari links semua note.
    fn relink_all(&mut self) {
        self.backlinks.clear();
        let rels: Vec<String> = self.notes.keys().cloned().collect();
        for rel in rels {
            let targets: Vec<String> = self
                .notes
                .get(&rel)
                .map(|n| n.links.iter().map(|l| l.target.clone()).collect())
                .unwrap_or_default();
            for t in targets {
                if let Resolved::Note(target) = self.resolve(&rel, &t) {
                    // self-link tidak dihitung sebagai backlink (perilaku Obsidian)
                    if target != rel {
                        self.backlinks.entry(target).or_default().insert(rel.clone());
                    }
                }
            }
        }
    }

    /// Hapus note dari indeks.
    pub fn remove(&mut self, rel_path: &str) -> &mut Self {
        if let Some(note) = self.notes.remove(rel_path) {
            let lpath = rel_path.trim_end_matches(".md").to_lowercase();
            self.by_lpath.remove(&lpath);
            if let Some(v) = self.by_title.get_mut(&note.title.to_lowercase()) {
                v.retain(|p| p != rel_path);
                if v.is_empty() {
                    self.by_title.remove(&note.title.to_lowercase());
                }
            }
            for tag in &note.tags {
                if let Some(v) = self.tag_map.get_mut(&tag.to_lowercase()) {
                    v.retain(|p| p != rel_path);
                    if v.is_empty() {
                        self.tag_map.remove(&tag.to_lowercase());
                    }
                }
            }
            // buang backlink entries yang menaut dari note ini
            // (resolve dari links note yang sudah diambil, karena note sudah dihapus dari map)
            for l in &note.links {
                if let Resolved::Note(t) = self.resolve(rel_path, &l.target) {
                    if let Some(s) = self.backlinks.get_mut(&t) {
                        s.remove(rel_path);
                    }
                }
            }
            self.backlinks.remove(rel_path);
        }
        self
    }

    /// Masukkan note ke map + peta judul/tag tanpa resolusi link.
    fn insert_raw(&mut self, note: NoteData) {
        let rel = note.rel_path.clone();
        let title_l = note.title.to_lowercase();
        let lpath = rel.trim_end_matches(".md").to_lowercase();
        self.by_lpath.insert(lpath, rel.clone());
        self.by_title.entry(title_l).or_default().push(rel.clone());
        for tag in &note.tags {
            self.tag_map
                .entry(tag.to_lowercase())
                .or_default()
                .push(rel.clone());
        }
        self.notes.insert(rel, note);
    }

    /// Semua rel_path note.
    pub fn paths(&self) -> Vec<String> {
        let mut v: Vec<String> = self.notes.keys().cloned().collect();
        v.sort();
        v
    }

    /// Referensi ke semua note (tanpa clone).
    pub fn iter_notes(&self) -> Vec<&NoteData> {
        self.notes.values().collect()
    }

    pub fn get(&self, rel_path: &str) -> Option<&NoteData> {
        self.notes.get(rel_path)
    }

    pub fn len(&self) -> usize {
        self.notes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }

    /// Resolve target link dari note sumber `from`.
    pub fn resolve(&self, from: &str, target: &str) -> Resolved {
        let target = target.trim();
        // 1) path relatif penuh (dengan atau tanpa .md)
        if let Some(p) = self.by_lpath.get(&target.trim_end_matches(".md").to_lowercase()) {
            return Resolved::Note(p.clone());
        }
        // 2) basename — prioritas satu folder dengan sumber, sisanya urut alfabet (stabil)
        let base = target.rsplit('/').next().unwrap_or(target);
        let base_l = base.trim_end_matches(".md").to_lowercase();
        if let Some(cands) = self.by_title.get(&base_l) {
            let from_dir = from.rsplit_once('/').map(|(d, _)| d);
            let pick = cands
                .iter()
                .min_by_key(|p| {
                    let pdir = p.rsplit_once('/').map(|(d, _)| d);
                    (pdir != from_dir, p.to_string()) // false < true → satu folder duluan
                })
                .unwrap();
            return Resolved::Note(pick.clone());
        }
        Resolved::Unresolved(target.to_string())
    }

    /// Backlinks note: daftar rel_path sumber (terurut).
    pub fn backlinks_of(&self, rel_path: &str) -> Vec<String> {
        let mut v: Vec<String> = self
            .backlinks
            .get(rel_path)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default();
        v.sort();
        v
    }

    /// Target keluar dari note (resolve saja, termasuk unresolved).
    pub fn outgoing(&self, rel_path: &str) -> Option<Vec<String>> {
        self.notes
            .get(rel_path)
            .map(|n| n.links.iter().map(|l| l.target.clone()).collect())
    }

    /// Peta tag → notes.
    pub fn tags(&self) -> &BTreeMap<String, Vec<String>> {
        &self.tag_map
    }

    /// Data graph penuh (node + edge resolved).
    pub fn graph(&self) -> (Vec<GraphNode>, Vec<Edge>) {
        let mut nodes = Vec::with_capacity(self.notes.len());
        let mut edge_set: HashSet<(String, String)> = HashSet::new();
        for (rel, note) in &self.notes {
            for l in &note.links {
                if let Resolved::Note(t) = self.resolve(rel, &l.target) {
                    if t != *rel {
                        edge_set.insert((rel.clone(), t));
                    }
                }
            }
        }
        let mut degree: HashMap<&str, usize> = HashMap::new();
        for (s, t) in &edge_set {
            *degree.entry(s.as_str()).or_default() += 1;
            *degree.entry(t.as_str()).or_default() += 1;
        }
        for (rel, note) in &self.notes {
            nodes.push(GraphNode {
                id: rel.clone(),
                title: note.title.clone(),
                link_count: *degree.get(rel.as_str()).unwrap_or(&0),
            });
        }
        nodes.sort_by(|a, b| a.id.cmp(&b.id));
        let mut edges: Vec<Edge> = edge_set
            .into_iter()
            .map(|(source, target)| Edge { source, target })
            .collect();
        edges.sort();
        (nodes, edges)
    }

    /// Daftar judul note (untuk autocomplete wikilink): (rel_path, title).
    pub fn all_titles(&self) -> Vec<(String, String)> {
        let mut v: Vec<(String, String)> = self
            .notes
            .values()
            .map(|n| (n.rel_path.clone(), n.title.clone()))
            .collect();
        v.sort();
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::parse_note;

    fn idx_from(pairs: &[(&str, &str)]) -> VaultIndex {
        VaultIndex::build(pairs.iter().map(|(p, r)| parse_note(p, r)).collect())
    }

    #[test]
    fn resolve_path_penuh_dan_basename() {
        let idx = idx_from(&[
            ("Rust/Notes.md", "x"),
            ("Java/Notes.md", "x"),
            ("Halo.md", "lihat [[Rust/Notes]] dan [[Halo]]"),
        ]);
        assert_eq!(
            idx.resolve("Halo.md", "Rust/Notes"),
            Resolved::Note("Rust/Notes.md".into())
        );
        assert_eq!(idx.resolve("Halo.md", "halo"), Resolved::Note("Halo.md".into()));
    }

    #[test]
    fn resolve_basename_ganda_prioritas_folder_sama() {
        let idx = idx_from(&[
            ("Rust/Notes.md", "x"),
            ("Java/Notes.md", "x"),
            ("Rust/Saya.md", "lihat [[Notes]]"),
        ]);
        assert_eq!(
            idx.resolve("Rust/Saya.md", "Notes"),
            Resolved::Note("Rust/Notes.md".into())
        );
    }

    #[test]
    fn resolve_unresolved() {
        let idx = idx_from(&[("a.md", "[[Nggak Ada]]")]);
        assert_eq!(
            idx.resolve("a.md", "Nggak Ada"),
            Resolved::Unresolved("Nggak Ada".into())
        );
    }

    #[test]
    fn backlink_dua_arah_index() {
        let idx = idx_from(&[
            ("a.md", "menuju [[B]]"),
            ("b.md", "balik ke [[a]] dan [[a|alias]]"),
        ]);
        assert_eq!(idx.backlinks_of("a.md"), vec!["b.md"]);
        assert_eq!(idx.backlinks_of("b.md"), vec!["a.md"]);
    }

    #[test]
    fn upsert_dan_remove_jaga_konsistensi() {
        let mut idx = idx_from(&[
            ("a.md", "ke [[B]] #t1"),
            ("b.md", "tenang"),
            ("c.md", "juga ke [[b]]"),
        ]);
        assert_eq!(idx.backlinks_of("b.md"), vec!["a.md", "c.md"]);
        // edit a.md: link hilang, tag berubah
        idx.upsert(parse_note("a.md", "sudah tidak menaut"));
        assert_eq!(idx.backlinks_of("b.md"), vec!["c.md"]);
        assert!(idx.tags().get("t1").is_none());
        // hapus c.md
        idx.remove("c.md");
        assert_eq!(idx.backlinks_of("b.md"), Vec::<String>::new());
        assert_eq!(idx.len(), 2);
    }

    #[test]
    fn graph_edges_resolved_saja() {
        let idx = idx_from(&[
            ("a.md", "ke [[b]] dan [[hantu]]"),
            ("b.md", "ke [[a]]"),
        ]);
        let (nodes, edges) = idx.graph();
        assert_eq!(nodes.len(), 2);
        let mut es: Vec<_> = edges.iter().map(|e| (e.source.as_str(), e.target.as_str())).collect();
        es.sort();
        assert_eq!(es, vec![("a.md", "b.md"), ("b.md", "a.md")]);
        // degree = keluar + masuk (a→b dan b→a)
        let a = nodes.iter().find(|n| n.id == "a.md").unwrap();
        assert_eq!(a.link_count, 2);
    }

    #[test]
    fn tags_map_case_insensitive() {
        let idx = idx_from(&[("a.md", "#Kerja"), ("b.md", "#kerja #Rumah")]);
        let t = idx.tags();
        assert_eq!(t.get("kerja").unwrap().len(), 2);
        assert_eq!(t.get("rumah").unwrap(), &vec!["b.md".to_string()]);
    }
}
