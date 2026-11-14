//! Where the app finds Stockfish, the Maia model, tablebases, the cache and saved results.

use std::path::PathBuf;

pub const MODEL_FILE: &str = "maia3-5m.history.fp32.onnx";

pub struct Paths {
    /// Per-user app data folder (e.g. %APPDATA%\app.trickiness.desktop).
    pub data_dir: PathBuf,
    /// Folder of the running executable (where Tauri puts the Stockfish sidecar).
    pub exe_dir: PathBuf,
}

fn first_existing(cands: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    cands.into_iter().find(|p| p.exists())
}

/// The repository root when running from a source checkout (`tauri dev`), for dev fallbacks.
fn dev_roots() -> Vec<PathBuf> {
    let cwd = std::env::current_dir().unwrap_or_default();
    vec![cwd.clone(), cwd.join(".."), cwd.join("../.."), cwd.join("../../..")]
}

impl Paths {
    /// Bundled sidecar next to the exe, else $TRICKINESS_STOCKFISH, else engines/ in a checkout.
    pub fn stockfish(&self) -> Option<PathBuf> {
        let exe = if cfg!(windows) { "stockfish.exe" } else { "stockfish" };
        let mut c = vec![self.exe_dir.join(exe)];
        if let Ok(p) = std::env::var("TRICKINESS_STOCKFISH") {
            c.push(p.into());
        }
        c.extend(dev_roots().into_iter().map(|r| r.join("engines").join(exe)));
        first_existing(c)
    }

    /// Where an installed or downloaded model goes.
    pub fn model_target(&self) -> PathBuf {
        self.data_dir.join("models").join(MODEL_FILE)
    }

    /// Installed model, else $TRICKINESS_MAIA, else models/ in a checkout.
    pub fn model(&self) -> Option<PathBuf> {
        let mut c = vec![self.model_target()];
        if let Ok(p) = std::env::var("TRICKINESS_MAIA") {
            c.push(p.into());
        }
        c.extend(dev_roots().into_iter().map(|r| r.join("models").join(MODEL_FILE)));
        first_existing(c)
    }

    /// Optional Syzygy folder (DECISIONS 2.11): $TRICKINESS_SYZYGY, else `tablebases` in the
    /// data folder, if it exists.
    pub fn tablebases(&self) -> Option<PathBuf> {
        let mut c = Vec::new();
        if let Ok(p) = std::env::var("TRICKINESS_SYZYGY") {
            c.push(PathBuf::from(p));
        }
        c.push(self.data_dir.join("tablebases"));
        c.into_iter().find(|p| p.is_dir())
    }

    pub fn cache_file(&self) -> PathBuf {
        self.data_dir.join("cache.sqlite")
    }

    /// Saved per-game results live next to the cache (SPEC Phase 3, step 5).
    pub fn results_dir(&self) -> PathBuf {
        self.data_dir.join("results")
    }
}
