//! File watcher vault: pantau perubahan eksternal (mis. editor lain /
//! sinkronisasi) dan broadcast event `vault-changed` ke frontend.
//! Debounce: kumpulkan event 250 ms, lalu emit sekali.

use notify::{Event, RecursiveMode, Watcher};
use std::path::Path;
use std::sync::mpsc::{sync_channel, Receiver, TryRecvError};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

/// Spawn thread watcher untuk `root`. Return channel untuk menghentikannya
/// (kirim `()`); thread exit saat sinyal diterima.
pub fn spawn(app: &AppHandle, root: &Path) -> Option<std::sync::mpsc::SyncSender<()>> {
    let (stop_tx, stop_rx) = sync_channel(1);
    let (tx, rx): (_, Receiver<Event>) = sync_channel(1024);
    let root = root.to_path_buf();
    let app = app.clone();

    let mut watcher = match notify::recommended_watcher(
        move |res: Result<Event, notify::Error>| {
        if let Ok(ev) = res {
            // abaikan event ACCESS (hasil pembacaan file) supaya scan/read
            // tidak memicu vault-changed → loop tak berujung
            let relevant = matches!(
                ev.kind,
                notify::EventKind::Modify(_) | notify::EventKind::Create(_) | notify::EventKind::Remove(_)
            );
            if relevant {
                let _ = tx.send(ev);
            }
        }
        },
    ) {
        Ok(w) => w,
        Err(_) => return None,
    };
    if watcher.watch(&root, RecursiveMode::Recursive).is_err() {
        return None;
    }

    std::thread::spawn(move || {
        // watcher harus hidup di thread ini
        let _watcher = watcher;
        loop {
            // cek stop
            match stop_rx.try_recv() {
                Ok(()) | Err(TryRecvError::Disconnected) => return,
                Err(TryRecvError::Empty) => {}
            }
            // BLOKIR menunggu event pertama (tanpa spin saat idle),
            // lalu kumpulkan event lanjutan selama debounce window.
            let mut changed: Vec<String> = Vec::new();
            while let Ok(ev) = rx.recv_timeout(Duration::from_millis(250)) {
                for p in ev.paths {
                    let rel = match p.strip_prefix(&root) {
                        Ok(r) => r.to_string_lossy().replace('\\', "/"),
                        Err(_) => continue,
                    };
                    // hanya file md, abaikan folder internal
                    if rel.ends_with(".md")
                        && !rel.split('/').any(|s| s.starts_with('.'))
                        && !changed.contains(&rel)
                    {
                        changed.push(rel);
                    }
                }
                if changed.len() >= 200 {
                    break;
                }
            }
            if !changed.is_empty() {
                let _ = app.emit(
                    "vault-changed",
                    serde_json::json!({ "self": false, "paths": changed }),
                );
            }
        }
    });
    Some(stop_tx)
}
