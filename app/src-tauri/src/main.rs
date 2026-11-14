//! Trickiness desktop app: starts the analysis core and streams its per-ply results to the
//! web front end. No chess math happens here or in the front end (SPEC.md Phase 3).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod paths;

use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use shakmaty::fen::Fen;
use shakmaty::san::SanPlus;
use shakmaty::uci::UciMove;
use shakmaty::{CastlingMode, Chess, Position};
use tauri::{AppHandle, Emitter, Manager, State};
use trickiness_core::analysis::{analyze_ply, PlyResult, Settings};
use trickiness_core::engines::EngineOracle;
use trickiness_core::game::{analyze_game, parse_all, Game};
use trickiness_core::oracle::{full_fen, EngineResult, HumanResult, Oracle};
use trickiness_core::practical::{practical, Practical, PracticalParams};
use trickiness_core::tree::Ratings;

use paths::Paths;

/// Hosted copy of our Maia ONNX export (tools/export_maia_onnx.py). Not decided yet
/// (DECISIONS 3.x); `TRICKINESS_MAIA_URL` overrides it.
const MAIA_MODEL_URL: Option<&str> = None;
/// SHA-256 of our export (models/maia3-5m.history.fp32.onnx), checked after download.
const MAIA_MODEL_SHA256: &str = "f2fc76df6749947d0325fa71e27a87fa0f065abd09b04b8ec01006571040e8a7";

struct AppState {
    paths: Paths,
    /// Engines, restarted when the node limit changes. (nodes, oracle)
    engines: Mutex<Option<(u64, Arc<EngineOracle>)>>,
    cancel: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    /// Bumped by every single-position request; an older request stops when it sees a newer one.
    explore_gen: Arc<AtomicU64>,
    /// The same for practical-move requests (separate, so the two don't cancel each other).
    practical_gen: Arc<AtomicU64>,
}

type CmdResult<T> = Result<T, String>;

fn err<E: std::fmt::Display>(e: E) -> String {
    format!("{e:#}")
}

// ---------------------------------------------------------------- status and model

#[derive(Serialize)]
struct Status {
    model_path: Option<String>,
    stockfish_path: Option<String>,
    tablebases: Option<String>,
    data_dir: String,
    download_url: Option<String>,
    running: bool,
}

#[tauri::command]
fn app_status(state: State<AppState>) -> Status {
    let p = &state.paths;
    Status {
        model_path: p.model().map(|x| x.display().to_string()),
        stockfish_path: p.stockfish().map(|x| x.display().to_string()),
        tablebases: p.tablebases().map(|x| x.display().to_string()),
        data_dir: p.data_dir.display().to_string(),
        download_url: model_url(),
        running: state.running.load(Ordering::SeqCst),
    }
}

fn model_url() -> Option<String> {
    std::env::var("TRICKINESS_MAIA_URL").ok().or(MAIA_MODEL_URL.map(String::from))
}

/// Size of the model download in bytes, so the user sees it before downloading.
#[tauri::command]
async fn model_download_size() -> CmdResult<u64> {
    let url = model_url().ok_or("The model's download location isn't set up yet. Use \"Locate model file\" instead.")?;
    tauri::async_runtime::spawn_blocking(move || {
        let resp = reqwest::blocking::Client::new().head(&url).send().map_err(err)?;
        if !resp.status().is_success() {
            return Err(format!("{url}: HTTP {}", resp.status()));
        }
        resp.content_length().ok_or_else(|| "the server didn't report a size".to_string())
    })
    .await
    .map_err(err)?
}

#[derive(Clone, Serialize)]
struct Progress {
    done: u64,
    total: u64,
}

#[tauri::command]
async fn download_model(app: AppHandle, state: State<'_, AppState>) -> CmdResult<String> {
    let url = model_url().ok_or("The model's download location isn't set up yet.")?;
    let dest = state.paths.model_target();
    tauri::async_runtime::spawn_blocking(move || -> CmdResult<String> {
        let mut resp = reqwest::blocking::Client::new().get(&url).send().map_err(err)?;
        if !resp.status().is_success() {
            return Err(format!("{url}: HTTP {}", resp.status()));
        }
        let total = resp.content_length().unwrap_or(0);
        fs::create_dir_all(dest.parent().unwrap()).map_err(err)?;
        let part = dest.with_extension("part");
        let mut out = File::create(&part).map_err(err)?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 1 << 16];
        let mut done = 0u64;
        loop {
            let n = resp.read(&mut buf).map_err(err)?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n]).map_err(err)?;
            hasher.update(&buf[..n]);
            done += n as u64;
            let _ = app.emit("model-progress", Progress { done, total });
        }
        drop(out);
        let hex: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if hex != MAIA_MODEL_SHA256 {
            let _ = fs::remove_file(&part);
            return Err(format!("the downloaded file's checksum doesn't match ({})", &hex[..12]));
        }
        fs::rename(&part, &dest).map_err(err)?;
        Ok(dest.display().to_string())
    })
    .await
    .map_err(err)?
}

/// Copy a model file the user picked into the app's data folder, after checking it loads.
#[tauri::command]
async fn import_model(path: String, state: State<'_, AppState>) -> CmdResult<String> {
    let dest = state.paths.model_target();
    tauri::async_runtime::spawn_blocking(move || -> CmdResult<String> {
        trickiness_core::maia::Maia::load(Path::new(&path), 1).map_err(err)?;
        fs::create_dir_all(dest.parent().unwrap()).map_err(err)?;
        fs::copy(&path, &dest).map_err(err)?;
        Ok(dest.display().to_string())
    })
    .await
    .map_err(err)?
}

// ---------------------------------------------------------------- PGN

#[derive(Serialize)]
struct MoveInfo {
    san: String,
    uci: String,
    /// Position after the move.
    fen: String,
}

#[derive(Serialize)]
struct GameInfo {
    index: usize,
    white: String,
    black: String,
    white_elo: Option<f32>,
    black_elo: Option<f32>,
    event: String,
    date: String,
    result: String,
    start_fen: String,
    moves: Vec<MoveInfo>,
    error: Option<String>,
}

fn game_info(index: usize, g: &Game) -> GameInfo {
    let tag = |n: &str| g.tag(n).unwrap_or("").to_string();
    GameInfo {
        index,
        white: tag("White"),
        black: tag("Black"),
        white_elo: g.elo("WhiteElo"),
        black_elo: g.elo("BlackElo"),
        event: tag("Event"),
        date: tag("Date"),
        result: tag("Result"),
        start_fen: full_fen(&g.positions[0]),
        moves: g
            .moves
            .iter()
            .zip(&g.positions[1..])
            .map(|((san, uci), pos)| MoveInfo { san: san.clone(), uci: uci.clone(), fen: full_fen(pos) })
            .collect(),
        error: None,
    }
}

/// Every game in a PGN text, for the game picker and the board.
#[tauri::command]
fn parse_pgn(text: String) -> CmdResult<Vec<GameInfo>> {
    let games = parse_all(&text).map_err(err)?;
    if games.is_empty() {
        return Err("No games found in this PGN.".into());
    }
    Ok(games
        .iter()
        .enumerate()
        .map(|(i, g)| match g {
            Ok(g) => game_info(i, g),
            Err(e) => GameInfo {
                index: i,
                white: String::new(),
                black: String::new(),
                white_elo: None,
                black_elo: None,
                event: String::new(),
                date: String::new(),
                result: String::new(),
                start_fen: String::new(),
                moves: vec![],
                error: Some(format!("{e:#}")),
            },
        })
        .collect())
}

#[tauri::command]
fn read_text_file(path: String) -> CmdResult<String> {
    let bytes = fs::read(&path).map_err(err)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

// ---------------------------------------------------------------- analysis

#[derive(Deserialize)]
struct AnalyzeRequest {
    pgn: String,
    game_index: usize,
    white_elo: f32,
    black_elo: f32,
    preset: String,
}

/// Presets settled in Phase 0 and Phase 2 (DECISIONS 0.8, 2.7).
fn preset(name: &str) -> CmdResult<(u32, f64, u64)> {
    match name {
        "quick" => Ok((1, 0.9, 50_000)),
        "standard" => Ok((2, 0.9, 50_000)),
        "deep" => Ok((2, 0.9, 200_000)),
        other => Err(format!("unknown preset {other}")),
    }
}

/// Stops the analysis between engine batches when the cancel flag is set.
struct Cancellable<'a> {
    inner: &'a EngineOracle,
    cancel: &'a AtomicBool,
}

impl Oracle for Cancellable<'_> {
    fn engine(&self, positions: &[Chess], multipv: usize) -> anyhow::Result<Vec<EngineResult>> {
        if self.cancel.load(Ordering::SeqCst) {
            anyhow::bail!("cancelled");
        }
        self.inner.engine(positions, multipv)
    }
    fn human(&self, histories: &[&[Chess]], es: f32, eo: f32) -> anyhow::Result<Vec<HumanResult>> {
        if self.cancel.load(Ordering::SeqCst) {
            anyhow::bail!("cancelled");
        }
        self.inner.human(histories, es, eo)
    }
}

fn engines_for(state: &AppState, nodes: u64) -> anyhow::Result<Arc<EngineOracle>> {
    let mut slot = state.engines.lock().unwrap();
    if let Some((n, o)) = slot.as_ref() {
        if *n == nodes {
            return Ok(o.clone());
        }
    }
    *slot = None; // stop the old processes before starting new ones
    let p = &state.paths;
    let sf = p.stockfish().ok_or_else(|| anyhow::anyhow!("Stockfish wasn't found next to the app."))?;
    let model = p.model().ok_or_else(|| anyhow::anyhow!("The Maia model isn't installed yet."))?;
    fs::create_dir_all(&p.data_dir)?;
    let workers = std::thread::available_parallelism().map(|n| (n.get() / 2).max(1)).unwrap_or(2);
    let o = Arc::new(EngineOracle::start(
        &sf,
        workers,
        nodes,
        16,
        p.tablebases().as_deref(),
        &model,
        Some(&p.cache_file()),
    )?);
    *slot = Some((nodes, o.clone()));
    Ok(o)
}

/// Settings for a run: the preset's k, p and nodes plus the approved ε, δ (DECISIONS 2.7, 3.9).
fn settings_for(k: u32, p: f64, nodes: u64, white_elo: f32, black_elo: f32, o: &EngineOracle) -> Settings {
    Settings {
        k,
        p,
        nodes,
        d: 0.5,
        epsilon: 0.01,
        delta: 0.05,
        multipv: 5,
        white_elo,
        black_elo,
        stockfish: o.stockfish.name.clone(),
        maia: o.maia.id.clone(),
    }
}

/// The saved-results file for a game + settings: changes to either give a new file.
fn results_key(g: &Game, s: &Settings) -> String {
    let mut h = Sha256::new();
    h.update(full_fen(&g.positions[0]).as_bytes());
    for (_, u) in &g.moves {
        h.update(u.as_bytes());
        h.update(b" ");
    }
    h.update(serde_json::to_string(s).unwrap().as_bytes());
    h.finalize().iter().take(16).map(|b| format!("{b:02x}")).collect()
}

#[derive(Clone, Serialize)]
struct Done {
    status: String, // "done" | "cancelled" | "error"
    message: Option<String>,
    from_saved: bool,
}

#[tauri::command]
fn start_analysis(req: AnalyzeRequest, app: AppHandle, state: State<AppState>) -> CmdResult<()> {
    if state.running.swap(true, Ordering::SeqCst) {
        return Err("An analysis is already running.".into());
    }
    state.cancel.store(false, Ordering::SeqCst);
    let result = (|| -> CmdResult<()> {
        let (k, p, nodes) = preset(&req.preset)?;
        let games = parse_all(&req.pgn).map_err(err)?;
        let game = games
            .into_iter()
            .nth(req.game_index)
            .ok_or("game not found")?
            .map_err(err)?;
        let oracle = engines_for(&state, nodes).map_err(err)?;
        let settings = settings_for(k, p, nodes, req.white_elo, req.black_elo, &oracle);
        let results_dir = state.paths.results_dir();
        let saved = results_dir.join(format!("{}.jsonl", results_key(&game, &settings)));
        let cancel = state.cancel.clone();
        let running = state.running.clone();
        std::thread::spawn(move || {
            let done = run_analysis(&app, &oracle, &game, &settings, &saved, &cancel);
            running.store(false, Ordering::SeqCst);
            let _ = app.emit("analysis-done", done);
        });
        Ok(())
    })();
    if result.is_err() {
        state.running.store(false, Ordering::SeqCst);
    }
    result
}

fn run_analysis(
    app: &AppHandle,
    oracle: &EngineOracle,
    game: &Game,
    settings: &Settings,
    saved: &Path,
    cancel: &AtomicBool,
) -> Done {
    // Saved results for exactly this game and these settings: replay them instantly.
    if let Ok(f) = File::open(saved) {
        let rows: Vec<Value> =
            BufReader::new(f).lines().map_while(Result::ok).filter_map(|l| serde_json::from_str(&l).ok()).collect();
        if rows.len() == game.positions.len() {
            for r in rows {
                let _ = app.emit("ply", r);
            }
            return Done { status: "done".into(), message: None, from_saved: true };
        }
    }
    let part = saved.with_extension("part");
    let outcome = (|| -> anyhow::Result<()> {
        fs::create_dir_all(saved.parent().unwrap())?;
        let mut out = File::create(&part)?;
        let c = Cancellable { inner: oracle, cancel };
        analyze_game(&c, game, settings, |r: &PlyResult| {
            writeln!(out, "{}", serde_json::to_string(r)?)?;
            let _ = app.emit("ply", r);
            Ok(!cancel.load(Ordering::SeqCst))
        })?;
        drop(out);
        if cancel.load(Ordering::SeqCst) {
            anyhow::bail!("cancelled");
        }
        fs::rename(&part, saved)?;
        Ok(())
    })();
    match outcome {
        Ok(()) => Done { status: "done".into(), message: None, from_saved: false },
        Err(e) => {
            let _ = fs::remove_file(&part);
            if cancel.load(Ordering::SeqCst) {
                Done { status: "cancelled".into(), message: None, from_saved: false }
            } else {
                Done { status: "error".into(), message: Some(format!("{e:#}")), from_saved: false }
            }
        }
    }
}

#[tauri::command]
fn cancel_analysis(state: State<AppState>) {
    state.cancel.store(true, Ordering::SeqCst);
}

// ---------------------------------------------------------------- playable board

fn parse_fen(fen: &str) -> CmdResult<Chess> {
    fen.parse::<Fen>()
        .map_err(err)?
        .into_position(CastlingMode::Standard)
        .map_err(err)
}

/// Legal moves (UCI) in a position, so the board only allows legal moves. The chess rules
/// live here, not in the front end.
#[tauri::command]
fn legal_moves(fen: String) -> CmdResult<Vec<String>> {
    let pos = parse_fen(&fen)?;
    Ok(pos.legal_moves().iter().map(|m| UciMove::from_move(m.clone(), CastlingMode::Standard).to_string()).collect())
}

/// Play a move: its SAN and the resulting position.
#[tauri::command]
fn play_move(fen: String, uci: String) -> CmdResult<MoveInfo> {
    let pos = parse_fen(&fen)?;
    let m = uci.parse::<UciMove>().map_err(err)?.to_move(&pos).map_err(err)?;
    let san = SanPlus::from_move(pos.clone(), m.clone()).to_string();
    let mut next = pos;
    next.play_unchecked(m);
    Ok(MoveInfo { san, uci, fen: full_fen(&next) })
}

#[derive(Deserialize)]
struct PositionRequest {
    /// FENs oldest..current (the last 8 matter to Maia).
    history: Vec<String>,
    ply: u32,
    /// (SAN, UCI) of the move that led here, if any.
    last_move: Option<(String, String)>,
    white_elo: f32,
    black_elo: f32,
    preset: String,
}

/// Stops a single-position analysis as soon as a newer request arrives.
struct Superseded<'a> {
    inner: &'a EngineOracle,
    gen: &'a AtomicU64,
    mine: u64,
}

impl Superseded<'_> {
    fn check(&self) -> anyhow::Result<()> {
        if self.gen.load(Ordering::SeqCst) != self.mine {
            anyhow::bail!("superseded");
        }
        Ok(())
    }
}

impl Oracle for Superseded<'_> {
    fn engine(&self, positions: &[Chess], multipv: usize) -> anyhow::Result<Vec<EngineResult>> {
        self.check()?;
        self.inner.engine(positions, multipv)
    }
    fn human(&self, histories: &[&[Chess]], es: f32, eo: f32) -> anyhow::Result<Vec<HumanResult>> {
        self.check()?;
        self.inner.human(histories, es, eo)
    }
}

/// Analyse one position (a side line or the new board). Shares the engines with a running
/// game analysis; a newer request makes this one return the error "superseded".
#[tauri::command]
async fn analyze_position(req: PositionRequest, app: AppHandle) -> CmdResult<PlyResult> {
    tauri::async_runtime::spawn_blocking(move || -> CmdResult<PlyResult> {
        let state = app.state::<AppState>();
        let mine = state.explore_gen.fetch_add(1, Ordering::SeqCst) + 1;
        let (k, p, nodes) = preset(&req.preset)?;
        // While a game analysis runs, reuse its engines rather than restarting them.
        // (Lock released before the match: engines_for takes the same lock.)
        let current = state.engines.lock().unwrap().clone();
        let oracle = match (state.running.load(Ordering::SeqCst), current) {
            (true, Some((_, o))) => o,
            _ => engines_for(&state, nodes).map_err(err)?,
        };
        let nodes = oracle.stockfish.nodes;
        let history: Vec<Chess> = req.history.iter().map(|f| parse_fen(f)).collect::<CmdResult<_>>()?;
        if history.is_empty() {
            return Err("empty history".into());
        }
        let settings = settings_for(k, p, nodes, req.white_elo, req.black_elo, &oracle);
        let guarded = Superseded { inner: &oracle, gen: &state.explore_gen, mine };
        let start = history.len().saturating_sub(trickiness_core::tree::HISTORY);
        analyze_ply(&guarded, &history[start..], req.ply, req.last_move, &settings).map_err(err)
    })
    .await
    .map_err(err)?
}

#[derive(Deserialize)]
struct PracticalRequest {
    /// FENs oldest..current.
    history: Vec<String>,
    white_elo: f32,
    black_elo: f32,
    preset: String,
    /// Value of a draw (the slider): 0.5 normally, 0 in a must-win game.
    d: f64,
}

/// Phase 4: objective vs. practical best move at one position (DECISIONS 4.x). Shares the
/// engines like `analyze_position`; a newer request makes this one return "superseded".
#[tauri::command]
async fn analyze_practical(req: PracticalRequest, app: AppHandle) -> CmdResult<Practical> {
    tauri::async_runtime::spawn_blocking(move || -> CmdResult<Practical> {
        let state = app.state::<AppState>();
        let mine = state.practical_gen.fetch_add(1, Ordering::SeqCst) + 1;
        let (_, p, nodes) = preset(&req.preset)?;
        let current = state.engines.lock().unwrap().clone();
        let oracle = match (state.running.load(Ordering::SeqCst), current) {
            (true, Some((_, o))) => o,
            _ => engines_for(&state, nodes).map_err(err)?,
        };
        let history: Vec<Chess> = req.history.iter().map(|f| parse_fen(f)).collect::<CmdResult<_>>()?;
        let params = PracticalParams { d: req.d.clamp(0.0, 1.0), p, ..PracticalParams::default() };
        let guarded = Superseded { inner: &oracle, gen: &state.practical_gen, mine };
        practical(&guarded, &history, Ratings { white: req.white_elo, black: req.black_elo }, params).map_err(err)
    })
    .await
    .map_err(err)?
}

// ---------------------------------------------------------------- export

fn csv_field(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) if s.contains([',', '"', '\n']) => format!("\"{}\"", s.replace('"', "\"\"")),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Write results as JSON (the full per-ply objects) or CSV (one flat row per ply).
#[tauri::command]
fn export_results(path: String, format: String, plies: Vec<Value>) -> CmdResult<()> {
    let mut out = File::create(&path).map_err(err)?;
    if format == "json" {
        serde_json::to_writer_pretty(&mut out, &plies).map_err(err)?;
        return Ok(());
    }
    let cols: [(&str, &[&str]); 24] = [
        ("ply", &["ply"]),
        ("san", &["san"]),
        ("uci", &["uci"]),
        ("fen", &["fen"]),
        ("side_to_move", &["side_to_move"]),
        ("v_star_white", &["v_star"]),
        ("draw_rate", &["draw_rate"]),
        ("t_white", &["trickiness", "white"]),
        ("t_black", &["trickiness", "black"]),
        ("stable_moves_white", &["diagnostics", "white", "stable_moves"]),
        ("trap_mass_white", &["diagnostics", "white", "trap_mass"]),
        ("stable_moves_black", &["diagnostics", "black", "stable_moves"]),
        ("trap_mass_black", &["diagnostics", "black", "trap_mass"]),
        ("k", &["settings", "k"]),
        ("p", &["settings", "p"]),
        ("nodes", &["settings", "nodes"]),
        ("d", &["settings", "d"]),
        ("epsilon", &["settings", "epsilon"]),
        ("delta", &["settings", "delta"]),
        ("multipv", &["settings", "multipv"]),
        ("white_elo", &["settings", "white_elo"]),
        ("black_elo", &["settings", "black_elo"]),
        ("stockfish", &["settings", "stockfish"]),
        ("maia", &["settings", "maia"]),
    ];
    writeln!(out, "{}", cols.iter().map(|c| c.0).collect::<Vec<_>>().join(",")).map_err(err)?;
    for row in &plies {
        let fields: Vec<String> = cols
            .iter()
            .map(|(_, path)| {
                let mut v = row;
                for k in *path {
                    v = v.get(k).unwrap_or(&Value::Null);
                }
                csv_field(v)
            })
            .collect();
        writeln!(out, "{}", fields.join(",")).map_err(err)?;
    }
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir: PathBuf = app.path().app_data_dir()?;
            let exe_dir = std::env::current_exe()?.parent().map(Path::to_path_buf).unwrap_or_default();
            app.manage(AppState {
                paths: Paths { data_dir, exe_dir },
                engines: Mutex::new(None),
                cancel: Arc::new(AtomicBool::new(false)),
                running: Arc::new(AtomicBool::new(false)),
                explore_gen: Arc::new(AtomicU64::new(0)),
                practical_gen: Arc::new(AtomicU64::new(0)),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_status,
            model_download_size,
            download_model,
            import_model,
            parse_pgn,
            read_text_file,
            start_analysis,
            cancel_analysis,
            export_results,
            legal_moves,
            play_move,
            analyze_position,
            analyze_practical
        ])
        .run(tauri::generate_context!())
        .expect("error while running the app");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_quotes_only_when_needed() {
        assert_eq!(csv_field(&Value::String("Nf3".into())), "Nf3");
        assert_eq!(csv_field(&Value::String("a,b".into())), "\"a,b\"");
        assert_eq!(csv_field(&serde_json::json!(0.25)), "0.25");
        assert_eq!(csv_field(&Value::Null), "");
    }

    #[test]
    fn board_commands_follow_the_rules() {
        let start = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1".to_string();
        assert_eq!(legal_moves(start.clone()).unwrap().len(), 20);
        let m = play_move(start.clone(), "g1f3".into()).unwrap();
        assert_eq!(m.san, "Nf3");
        assert!(m.fen.starts_with("rnbqkbnr/pppppppp/8/8/8/5N2/PPPPPPPP/RNBQKB1R b"));
        assert!(play_move(start, "e2e5".into()).is_err());
        // Promotion needs the piece letter.
        let promo = "8/P6k/8/8/8/8/8/K7 w - - 0 1".to_string();
        assert!(legal_moves(promo.clone()).unwrap().contains(&"a7a8q".to_string()));
        assert_eq!(play_move(promo, "a7a8n".into()).unwrap().san, "a8=N");
    }

    #[test]
    fn presets_match_decisions() {
        assert_eq!(preset("standard").unwrap(), (2, 0.9, 50_000));
        assert!(preset("other").is_err());
    }

    #[test]
    fn results_key_changes_with_settings() {
        let g = trickiness_core::game::parse_pgn("1. e4 e5 *", 0).unwrap();
        let mut s = Settings {
            k: 2, p: 0.9, nodes: 50_000, d: 0.5, epsilon: 0.01, delta: 0.05, multipv: 5,
            white_elo: 1500.0, black_elo: 1500.0, stockfish: "sf".into(), maia: "m".into(),
        };
        let a = results_key(&g, &s);
        s.white_elo = 1600.0;
        assert_ne!(a, results_key(&g, &s));
    }
}
