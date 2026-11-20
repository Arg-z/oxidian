//! Parser `[[wikilink]]` dan `#tag` mengikuti sintaks Obsidian yang terdokumentasi:
//! - `[[Note]]`, `[[Note|alias]]`, `[[Note#heading]]`, `[[folder/Note#h|alias]]`
//! - embed `![[Note]]` ikut dihitung sebagai link
//! - `#tag` / `#tag/bersarang` di luar code fence & inline code

use serde::{Deserialize, Serialize};

/// Satu wikilink mentah hasil parsing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawLink {
    /// Target persis seperti ditulis user, mis. `Rust/Notes` atau `catatan`.
    pub target: String,
    /// Bagian setelah `#` (opsional).
    pub heading: Option<String>,
    /// Alias setelah `|` (opsional) — teks yang ditampilkan.
    pub alias: Option<String>,
    /// Span byte `[start, end)` dari keseluruhan `[[...]]` pada konten.
    /// Valid karena [`strip_code`] mempertahankan panjang string.
    pub start: usize,
    pub end: usize,
}

impl RawLink {
    /// Teks yang sebaiknya ditampilkan (alias, atau target).
    pub fn display(&self) -> &str {
        self.alias.as_deref().unwrap_or(&self.target)
    }
}

/// "Hapus" code fence (``` ... ```) dan inline code (` ... `) supaya
/// isi di dalamnya tidak ikut diberi link/tag (perilaku Obsidian).
/// **Mempertahankan panjang string** — setiap karakter non-newline di dalam
/// code diganti spasi, sehingga span link tetap valid pada string asli.
pub fn strip_code(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut in_fence = false;
    for line in content.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
        }
        if in_fence {
            // seluruh baris (di dalam fence atau baris marker) → spasi, newline tetap
            for c in line.chars() {
                out.push(if c == '\n' { '\n' } else { ' ' });
            }
        } else {
            // inline code: isi antara backtick → spasi, backtick tetap
            let mut in_inline = false;
            for c in line.chars() {
                if c == '`' {
                    in_inline = !in_inline;
                    out.push('`');
                } else {
                    out.push(if in_inline { ' ' } else { c });
                }
            }
        }
    }
    out
}

/// Parse semua wikilink dari konten note.
pub fn parse_links(content: &str) -> Vec<RawLink> {
    let clean = strip_code(content);
    let bytes = clean.as_bytes();
    let mut links = Vec::new();
    let mut i = 0;
    while i + 1 < bytes.len() {
        // mulai hanya pada "[[" — "![[embed]]" otomatis ketemu karena '!' di depan
        if bytes[i] == b'[' && bytes[i + 1] == b'[' {
            if let Some((link, end)) = parse_one(&clean, i + 2) {
                links.push(link);
                i = end;
                continue;
            }
        }
        i += 1;
    }
    links
}

/// Parse `... ]]` mulai dari `start` (setelah `[[`).
/// Return (link, index setelah `]]`).
fn parse_one(s: &str, start: usize) -> Option<(RawLink, usize)> {
    let b = s.as_bytes();
    let mut i = start;
    // badan link tidak boleh mengandung '[' , ']' , atau newline
    while i < b.len() && b[i] != b']' && b[i] != b'[' && b[i] != b'\n' {
        i += 1;
    }
    if i + 1 >= b.len() || b[i] != b']' || b[i + 1] != b']' {
        return None; // tidak ketutup — bukan link valid
    }
    let body = &s[start..i];
    if body.trim().is_empty() {
        return None;
    }
    // pisah alias: [[target#heading|alias]] — alias boleh kosong ( [[a|]] )
    let (main, alias) = match body.find('|') {
        Some(p) => (&body[..p], Some(body[p + 1..].trim().to_string())),
        None => (body, None),
    };
    // pisah heading
    let (target, heading) = match main.find('#') {
        Some(p) => (main[..p].trim(), Some(main[p + 1..].trim().to_string())),
        None => (main.trim(), None),
    };
    if target.is_empty() && heading.is_none() {
        return None;
    }
    Some((
        RawLink {
            target: target.to_string(),
            heading: heading.filter(|h| !h.is_empty()),
            alias: alias.filter(|a| !a.is_empty()),
            start: start.saturating_sub(2), // posisi '[' pertama
            end: i + 2,
        },
        i + 2,
    ))
}

/// Parse semua `#tag` (dan `#tag/bersarang`) di luar code.
pub fn parse_tags(content: &str) -> Vec<String> {
    let clean = strip_code(content);
    let mut tags = Vec::new();
    let bytes = clean.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        let prev_is_boundary = i == 0
            || matches!(bytes[i - 1], b' ' | b'\t' | b'\n' | b'(' | b'[' | b'{' | b'"' | b'\'');
        if c == b'#' && prev_is_boundary {
            let mut j = i + 1;
            while j < bytes.len()
                && (bytes[j].is_ascii_alphanumeric()
                    || bytes[j] == b'_'
                    || bytes[j] == b'-'
                    || bytes[j] == b'/')
            {
                j += 1;
            }
            // tag valid minimal 1 char dan bukan "#saja" (angka semua = heading-ish, tapi Obsidian tetap; kita terima huruf/angka)
            if j > i + 1 {
                tags.push(clean[i + 1..j].to_string());
                i = j;
                continue;
            }
        }
        // '#' yang tidak di boundary (mis. "C#") otomatis dilewati
        i += 1;
    }
    // dedup order-preserving (dedup() hanya buang duplikat berurutan)
    let mut seen = std::collections::HashSet::new();
    tags.retain(|t| seen.insert(t.clone()));
    tags
}

/// Gabungkan tag dari frontmatter (list atau string) dengan tag inline.
pub fn merge_frontmatter_tags(fm_tags: Option<&serde_json::Value>, inline: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(v) = fm_tags {
        match v {
            serde_json::Value::Array(a) => {
                for t in a {
                    if let Some(s) = t.as_str() {
                        push_unique(&mut out, s);
                    }
                }
            }
            serde_json::Value::String(s) => {
                for t in s.split(',') {
                    push_unique(&mut out, t.trim().trim_start_matches('#'));
                }
            }
            _ => {}
        }
    }
    for t in inline {
        push_unique(&mut out, t);
    }
    out
}

fn push_unique(v: &mut Vec<String>, s: &str) {
    let s = s.trim();
    if !s.is_empty() && !v.iter().any(|x| x.eq_ignore_ascii_case(s)) {
        v.push(s.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_link() {
        let l = parse_links("lihat [[Catatan Penting]] ya");
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].target, "Catatan Penting");
        assert_eq!(l[0].alias, None);
        assert_eq!(l[0].heading, None);
    }

    #[test]
    fn link_alias_dan_heading() {
        let l = parse_links("[[Rust/Notes#Safety|safety notes]] dan [[A#B]]");
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].target, "Rust/Notes");
        assert_eq!(l[0].heading.as_deref(), Some("Safety"));
        assert_eq!(l[0].alias.as_deref(), Some("safety notes"));
        assert_eq!(l[1].target, "A");
        assert_eq!(l[1].heading.as_deref(), Some("B"));
    }

    #[test]
    fn embed_dan_unclosed() {
        let l = parse_links("![[gambar]] dan [[yang tidak ketutup");
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].target, "gambar");
    }

    #[test]
    fn link_dalam_code_dihapus() {
        let l = parse_links("teks\n```rust\nlet x = \"[[bukan link]]\";\n```\n[[yang asli]]");
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].target, "yang asli");
    }

    #[test]
    fn strip_code_mempertahankan_panjang() {
        let s = "awal `[[kode]]` sini\n```rust\nlet x = 1;\n```\nakhir [[Nyata]]\n";
        let stripped = strip_code(s);
        assert_eq!(stripped.len(), s.len());
        // span link valid pada string asli
        let links = parse_links(s);
        assert_eq!(links.len(), 1);
        let l = &links[0];
        assert_eq!(&s[l.start..l.end], "[[Nyata]]");
    }

    #[test]
    fn span_meliputi_alias_dan_heading() {
        let s = "lihat [[Rust/Lama#Panduan|bacaan]] sekarang";
        let l = &parse_links(s)[0];
        assert_eq!(&s[l.start..l.end], "[[Rust/Lama#Panduan|bacaan]]");
    }

    #[test]
    fn tags_duplikat_dedupe_order_preserving() {
        let t = parse_tags("#a #b #a #c #b");
        assert_eq!(t, vec!["a", "b", "c"]);
    }

    #[test]
    fn tags_dasar() {
        let t = parse_tags("#kerja dan #project/oxidian, tapi C# bukan tag, # juga bukan");
        assert_eq!(t, vec!["kerja", "project/oxidian"]);
    }

    #[test]
    fn tags_heading_tidak_ikut() {
        // "# Judul" punya spasi setelah '#' → bukan tag
        let t = parse_tags("# Judul dokumen\n\nisi #nyata");
        assert_eq!(t, vec!["nyata"]);
    }

    #[test]
    fn tags_dalam_code_dihapus() {
        let t = parse_tags("```\n#bukan\n```\n`#inline juga bukan`\n#iya");
        assert_eq!(t, vec!["iya"]);
    }

    #[test]
    fn fm_tags_gabung() {
        let fm = serde_json::json!(["alpha", "beta"]);
        let mut inline = vec!["beta".to_string(), "gamma".to_string()];
        let merged = merge_frontmatter_tags(Some(&fm), &inline);
        assert_eq!(merged, vec!["alpha", "beta", "gamma"]);
        inline.clear();
    }

    #[test]
    fn link_case_tetap_seperti_ditulis() {
        let l = parse_links("[[catatan Kecil]]");
        assert_eq!(l[0].target, "catatan Kecil");
    }
}
