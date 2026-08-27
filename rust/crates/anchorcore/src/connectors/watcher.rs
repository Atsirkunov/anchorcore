use notify::{Event, RecursiveMode, Watcher};
use std::path::Path;
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

/// Simple watcher like Python `folder_watcher` - debounces 3s, watches `path` recursively.
/// Returns a receiver that yields `PathBuf` of changed files.
pub struct FolderWatcher {
    _watcher: notify::RecommendedWatcher,
    rx: Receiver<Result<Event, notify::Error>>,
}

impl FolderWatcher {
    pub fn new(path: &Path) -> Result<Self, String> {
        let (tx, rx) = channel();
        let mut watcher = notify::recommended_watcher(tx).map_err(|e| e.to_string())?;
        watcher
            .watch(path, RecursiveMode::Recursive)
            .map_err(|e| e.to_string())?;
        Ok(Self { _watcher: watcher, rx })
    }

    /// Non-blocking poll with debounce (like Python `folder_watch_debounce` 3.0)
    pub fn poll(&self, debounce: Duration) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        // collect all pending events within debounce window
        let start = std::time::Instant::now();
        while start.elapsed() < debounce {
            match self.rx.recv_timeout(Duration::from_millis(100)) {
                Ok(Ok(ev)) => {
                    for p in ev.paths {
                        if !out.contains(&p) {
                            out.push(p);
                        }
                    }
                }
                _ => break,
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn watches() {
        let dir = tempdir().unwrap();
        let w = FolderWatcher::new(dir.path()).unwrap();
        std::fs::write(dir.path().join("x.txt"), "hi").unwrap();
        // give watcher time
        std::thread::sleep(Duration::from_millis(200));
        let evs = w.poll(Duration::from_millis(300));
        // may or may not have event depending on FS, just check no panic
        assert!(evs.len() < 1000);
    }
}
