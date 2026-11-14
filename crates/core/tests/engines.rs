//! Tests against the real engines. They skip (with a message) when the model or Stockfish
//! isn't present: models/maia3-5m.history.fp32.onnx and engines/stockfish(.exe), or the
//! TRICKINESS_MAIA / TRICKINESS_STOCKFISH environment variables.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::Deserialize;
use shakmaty::{fen::Fen, uci::UciMove, CastlingMode, Chess, Position};
use trickiness_core::maia::Maia;
use trickiness_core::stockfish::StockfishPool;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn maia_path() -> Option<PathBuf> {
    let p = std::env::var("TRICKINESS_MAIA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| root().join("models/maia3-5m.history.fp32.onnx"));
    if p.exists() {
        return Some(p);
    }
    eprintln!("skipping: no Maia model at {}", p.display());
    None
}

fn stockfish_path() -> Option<PathBuf> {
    let p = std::env::var("TRICKINESS_STOCKFISH").map(PathBuf::from).unwrap_or_else(|_| {
        root().join(if cfg!(windows) { "engines/stockfish.exe" } else { "engines/stockfish" })
    });
    if p.exists() {
        return Some(p);
    }
    eprintln!("skipping: no Stockfish at {}", p.display());
    None
}

#[derive(Deserialize)]
struct Case {
    name: String,
    start_fen: Option<String>,
    moves: Vec<String>,
    elo: f32,
    probs: HashMap<String, f64>,
    wdl: [f64; 3],
}

/// Phase 0 reference outputs from the official Python package (spikes/s1_reference.json):
/// 14 positions × 3 ratings, with real move history.
#[test]
fn maia_matches_official_package() {
    let Some(model) = maia_path() else { return };
    let refs = root().join("spikes/s1_reference.json");
    let cases: Vec<Case> = serde_json::from_str(&std::fs::read_to_string(refs).unwrap()).unwrap();
    let maia = Maia::load(&model, 1).unwrap();
    let mut worst_p: f64 = 0.0;
    let mut worst_v: f64 = 0.0;
    for c in &cases {
        let start: Chess = match &c.start_fen {
            Some(f) => f.parse::<Fen>().unwrap().into_position(CastlingMode::Standard).unwrap(),
            None => Chess::default(),
        };
        let mut hist = vec![start];
        for u in &c.moves {
            let mut p = hist.last().unwrap().clone();
            p.play_unchecked(u.parse::<UciMove>().unwrap().to_move(&p).unwrap());
            hist.push(p);
        }
        let out = maia.evaluate(&[&hist], c.elo, c.elo).unwrap().remove(0);
        assert_eq!(out.moves.len(), c.probs.len(), "{}: legal move count", c.name);
        for (u, p) in &out.moves {
            worst_p = worst_p.max((p - c.probs[u]).abs());
        }
        let w = [out.wdl.win, out.wdl.draw, out.wdl.loss];
        for i in 0..3 {
            worst_v = worst_v.max((w[i] - c.wdl[i]).abs());
        }
    }
    eprintln!("max differences vs official package: move prob {worst_p:.2e}, wdl {worst_v:.2e}");
    assert!(worst_p < 1e-4 && worst_v < 1e-4);
}

/// Batching must not change results.
#[test]
fn maia_batch_equals_single() {
    let Some(model) = maia_path() else { return };
    let maia = Maia::load(&model, 1).unwrap();
    let a = Chess::default();
    let mut b = a.clone();
    b.play_unchecked("e2e4".parse::<UciMove>().unwrap().to_move(&b).unwrap());
    let ha = vec![a.clone()];
    let hb = vec![a, b];
    let both = maia.evaluate(&[&ha, &hb], 1500.0, 1500.0).unwrap();
    let single = maia.evaluate(&[&hb], 1500.0, 1500.0).unwrap();
    for ((m1, p1), (m2, p2)) in both[1].moves.iter().zip(&single[0].moves) {
        assert_eq!(m1, m2);
        assert!((p1 - p2).abs() < 1e-6);
    }
}

/// With a cleared hash before each search, repeating a search gives the same answer
/// (DECISIONS 0.10), across processes too.
#[test]
fn stockfish_searches_are_reproducible() {
    let Some(sf) = stockfish_path() else { return };
    let pool = StockfishPool::start(&sf, 2, 20_000, 16, None).unwrap();
    let fens = [
        "r1bqkb1r/pppp1ppp/2n2n2/4p2Q/2B1P3/8/PPPP1PPP/RNB1K1NR w KQkq - 4 4",
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
        "8/7r/3k4/8/4K3/3B4/8/5R2 b - - 0 1",
    ];
    let positions: Vec<Chess> = fens
        .iter()
        .map(|f| f.parse::<Fen>().unwrap().into_position(CastlingMode::Standard).unwrap())
        .collect();
    // The second run goes in reverse order, so each position lands on a different process
    // after different earlier searches.
    let first = pool.search_many(&positions, 3).unwrap();
    let rev: Vec<Chess> = positions.iter().rev().cloned().collect();
    let mut second = pool.search_many(&rev, 3).unwrap();
    second.reverse();
    let runs = [first, second];
    for (a, b) in runs[0].iter().zip(&runs[1]) {
        assert_eq!(a.lines.len(), b.lines.len());
        for (x, y) in a.lines.iter().zip(&b.lines) {
            assert_eq!(x.uci, y.uci);
            assert_eq!(x.wdl, y.wdl);
        }
    }
    // Scholar's mate threat: Qxf7# is found and scored as a win.
    assert_eq!(runs[0][0].lines[0].uci, "h5f7");
    assert!(runs[0][0].lines[0].wdl.win > 0.99);
}

/// A folder with no tablebases must not change the engine identity (cache keys, output).
#[test]
fn syzygy_empty_folder_is_ignored() {
    let Some(sf) = stockfish_path() else { return };
    let dir = std::env::temp_dir().join(format!("trickiness-empty-tb-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let plain = StockfishPool::start(&sf, 1, 1_000, 16, None).unwrap();
    let empty = StockfishPool::start(&sf, 1, 1_000, 16, Some(&dir)).unwrap();
    assert_eq!(plain.name, empty.name);
    assert!(!empty.name.contains("Syzygy"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// With real tablebases (TRICKINESS_SYZYGY), the name records them and a won K+R vs K is
/// scored as an exact win even at a tiny node count.
#[test]
fn syzygy_tables_are_used_when_present() {
    let Some(sf) = stockfish_path() else { return };
    let Ok(tb) = std::env::var("TRICKINESS_SYZYGY") else {
        eprintln!("skipping: TRICKINESS_SYZYGY not set");
        return;
    };
    let pool = StockfishPool::start(&sf, 1, 100, 16, Some(std::path::Path::new(&tb))).unwrap();
    assert!(pool.name.contains("Syzygy"), "{}", pool.name);
    let pos: Chess = "8/8/8/4k3/8/8/8/R3K3 w - - 0 1".parse::<Fen>().unwrap().into_position(CastlingMode::Standard).unwrap();
    let r = pool.search_many(&[pos], 1).unwrap();
    assert!(r[0].lines[0].wdl.win > 0.999);
}
