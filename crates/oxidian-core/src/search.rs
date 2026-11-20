//! Pencarian in-memory: judul (fuzzy subsequence) + tag + konten (dengan snippet).
//! Cukup cepat untuk ribuan note tanpa dependensi index eksternal.

use crate::vault::NoteData;
use serde::{Deserialize, Serialize};

/// Satu hasil pencarian.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub rel_path: String,
    pub title: String,
    pub score: i64,
    /// Cuplikan konten di sekitar kecocokan pertama (jika ada).
    pub snippet: String,
    /// true jika kecocokan di judul/tag (untuk quick switcher).
    pub title_match: bool,
}

/// Cari di seluruh vault.
/// `query` kosong → return semua note urut judul (untuk quick switcher).
pub fn search<'a, I>(notes: I, query: &str) -> Vec<SearchHit>
where
    I: IntoIterator<Item = &'a NoteData>,
{
    let q = query.trim();
    if q.is_empty() {
        let mut hits: Vec<SearchHit> = notes
            .into_iter()
            .map(|n| SearchHit {
                rel_path: n.rel_path.clone(),
                title: n.title.clone(),
                score: 0,
                snippet: first_line(&n.body),
                title_match: true,
            })
            .collect();
        hits.sort_by_key(|a| a.title.to_lowercase());
        return hits;
    }
    let q_lower = q.to_lowercase();
    let mut hits = Vec::new();
    for n in notes.into_iter() {
        let title_l = n.title.to_lowercase();
        let mut score: i64 = 0;
        let mut title_match = false;

        if title_l == q_lower {
            score += 100;
            title_match = true;
        } else if title_l.starts_with(&q_lower) {
            score += 60;
            title_match = true;
        } else if title_l.contains(&q_lower) {
            score += 40;
            title_match = true;
        } else if let Some(s) = subseq_score(&title_l, &q_lower) {
            score += 20 + s;
            title_match = true;
        }

        // tag
        let tag_hit = n.tags.iter().any(|t| t.to_lowercase().contains(&q_lower));
        if tag_hit {
            score += 25;
            title_match = true;
        }

        // konten
        let body_l = n.body.to_lowercase();
        let occurrences = body_l.matches(&q_lower).count() as i64;
        if occurrences > 0 {
            score += 5 * occurrences.min(10);
        }

        if score == 0 {
            continue;
        }
        let snippet = snippet_around(&n.body, &q_lower);
        hits.push(SearchHit {
            rel_path: n.rel_path.clone(),
            title: n.title.clone(),
            score,
            snippet,
            title_match,
        });
    }
    hits.sort_by(|a, b| b.score.cmp(&a.score).then(a.title.to_lowercase().cmp(&b.title.to_lowercase())));
    hits.truncate(50);
    hits
}

/// Skor subsequence fuzzy: semua char query muncul berurutan di target.
/// Bonus untuk run berurutan (kontigu), penalti jarak.
fn subseq_score(target: &str, query: &str) -> Option<i64> {
    let t: Vec<char> = target.chars().collect();
    let mut ti = 0;
    let mut score = 0i64;
    let mut last: Option<usize> = None;
    for qc in query.chars() {
        let found = t[ti..].iter().position(|&c| c == qc)? + ti;
        if let Some(l) = last {
            score += if found == l + 1 { 5 } else { 0 };
            score -= ((found - l) as i64 / 4).min(3);
        }
        last = Some(found);
        ti = found + 1;
        if ti >= t.len() && qc != query.chars().last().unwrap() {
            return None;
        }
    }
    Some(score.max(0))
}

fn first_line(body: &str) -> String {
    body.lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && *l != "---")
        .unwrap_or("")
        .chars()
        .take(80)
        .collect()
}

/// Potongan ±60 karakter di sekitar kecocokan pertama (case-insensitive).
fn snippet_around(body: &str, q_lower: &str) -> String {
    let lower = body.to_lowercase();
    let Some(pos) = lower.find(q_lower) else {
        return first_line(body);
    };
    let mut start = pos.saturating_sub(40);
    // geser ke word boundary supaya tidak terpotong di tengah kata
    if let Some(o) = body[start..].find(' ') {
        start += o + 1;
    }
    let end = (pos + q_lower.len() + 60).min(body.len());
    let s: String = body[start..end].replace(['\n', '\r'], " ");
    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    out.push_str(&s);
    if end < body.len() {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::parse_note;

    fn notes(pairs: &[(&str, &str)]) -> Vec<NoteData> {
        pairs.iter().map(|(p, r)| parse_note(p, r)).collect()
    }

    #[test]
    fn query_kosong_return_semua() {
        let ns = notes(&[("b.md", "x"), ("a.md", "y")]);
        let hits = search(&ns, "");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].title, "a");
    }

    #[test]
    fn judul_exact_lebih_tinggi_dari_konten() {
        let ns = notes(&[
            ("rust.md", "belajar rust bahasa"),
            ("catatan.md", "sangat banyak bicara rust rust rust di sini"),
        ]);
        let hits = search(&ns, "rust");
        assert_eq!(hits[0].title, "rust");
    }

    #[test]
    fn fuzzy_subsequence_di_judul() {
        let ns = notes(&[("Oxidian App.md", "isi biasa")]);
        let hits = search(&ns, "oxa");
        assert_eq!(hits.len(), 1);
        assert!(hits[0].title_match);
    }

    #[test]
    fn snippet_berisi_konteks() {
        let ns = notes(&[("a.md", "panjang sekali teksnya dan kunci rahasia ada di tengah sini")]);
        let hits = search(&ns, "rahasia");
        assert!(hits[0].snippet.contains("rahasia"));
    }

    #[test]
    fn tag_kebaca() {
        let ns = notes(&[("a.md", "#project"), ("b.md", "biasa saja")]);
        let hits = search(&ns, "project");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].rel_path, "a.md");
    }
}
