//! oxidian-core: vault engine yang kompatibel dengan vault Markdown Obsidian.
//!
//! Semua logika murni (tanpa I/O GUI) supaya bisa di-unit-test:
//! - [`links`]   : parser `[[wikilink]]` dan `#tag`
//! - [`vault`]   : scan vault + model [`vault::NoteData`]
//! - [`index`]   : [`index::VaultIndex`] — resolusi link, backlinks, tags, graph
//! - [`search`]  : scorer pencarian fuzzy judul + konten
//! - [`config`]  : config aplikasi (`~/.config/oxidian/config.json`)

pub mod config;
pub mod index;
pub mod links;
pub mod rename;
pub mod search;
pub mod vault;

pub use index::VaultIndex;
pub use vault::NoteData;
