//! Phase 2 validation runs. Writes raw results; the checks and plots are in validation/*.py.
//!
//!   validate sanity --positions validation/positions.yaml --out validation/results/sanity.json
//!   validate sample --games validation/data/band_1400_1600.pgn --limit 600 --out validation/results/t_1400_1600.jsonl
//!   validate draws  --games validation/data/band_1400_1600.pgn --out validation/results/d_1400_1600.jsonl

use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{anyhow, Context};
use clap::{Args, Parser, Subcommand};
use serde::{Deserialize, Serialize};
use serde_json::json;
use shakmaty::{fen::Fen, san::San, CastlingMode, Chess, Color, Position};
use trickiness_core::analysis::decision_inputs;
use trickiness_core::engines::EngineOracle;
use trickiness_core::game::{parse_all, Game};
use trickiness_core::oracle::{full_fen, Oracle};
use trickiness_core::tree::{self, terminal_value, Ratings, TreeParams, HISTORY};

const D: f64 = 0.5;

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Args)]
struct Engines {
    #[arg(long, default_value = "engines/stockfish.exe")]
    stockfish: PathBuf,
    #[arg(long, default_value = "models/maia3-5m.history.fp32.onnx")]
    maia_model: PathBuf,
    #[arg(long, default_value_t = 50_000)]
    nodes: u64,
    #[arg(long, default_value_t = 4)]
    workers: usize,
    #[arg(long, default_value = "trickiness-cache.sqlite")]
    cache: PathBuf,
    /// Syzygy tablebase folder (used if Stockfish finds tables in it).
    #[arg(long, default_value = "tablebases")]
    syzygy: PathBuf,
}

impl Engines {
    fn start(&self) -> anyhow::Result<EngineOracle> {
        let tb = self.syzygy.is_dir().then_some(self.syzygy.as_path());
        EngineOracle::start(&self.stockfish, self.workers, self.nodes, 16, tb, &self.maia_model, Some(&self.cache))
    }
}

#[derive(Subcommand)]
enum Cmd {
    /// T and D for the sanity positions at several ratings (both players at the same rating).
    Sanity {
        #[arg(long)]
        positions: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, value_delimiter = ',', default_value = "1100,1500,1900")]
        ratings: Vec<f32>,
        #[arg(long, value_delimiter = ',', default_value = "1,2")]
        ks: Vec<u32>,
        #[arg(long, default_value_t = 0.9)]
        p: f64,
        #[command(flatten)]
        eng: Engines,
    },
    /// T test sample: one position per game (the [SamplePly] header), T over a (k, p) grid,
    /// the diagnostic inputs, and v* along the next plies for realized loss. Resumable.
    Sample {
        #[arg(long)]
        games: PathBuf,
        #[arg(long)]
        out: PathBuf,
        /// Use only the first N games.
        #[arg(long)]
        limit: Option<usize>,
        #[command(flatten)]
        eng: Engines,
    },
    /// D test: Maia's D and Stockfish's W/D/L at a fixed ply for every game.
    Draws {
        #[arg(long)]
        games: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value_t = 30)]
        ply: usize,
        #[command(flatten)]
        eng: Engines,
    },
    /// Leaf calibration data (DECISIONS 4.5): Stockfish's W/D/L at several plies of every game,
    /// with the ratings, material and the game's result.
    Evals {
        #[arg(long)]
        games: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, value_delimiter = ',', default_value = "12,24,36,48")]
        plies: Vec<usize>,
        #[command(flatten)]
        eng: Engines,
    },
}

#[derive(Deserialize)]
struct PosSpec {
    id: String,
    category: String,
    fen: Option<String>,
    moves: Option<String>,
    #[serde(default)]
    roles: std::collections::BTreeMap<String, String>,
}

#[derive(Serialize)]
struct SanityRow {
    id: String,
    category: String,
    roles: std::collections::BTreeMap<String, String>,
    fen: String,
    rating: f32,
    k: u32,
    p: f64,
    v_star_white: f64,
    t_white: f64,
    t_black: f64,
    draw_rate: f64,
}

fn history_of(spec: &PosSpec) -> anyhow::Result<Vec<Chess>> {
    let start: Chess = match &spec.fen {
        Some(f) => f.parse::<Fen>()?.into_position(CastlingMode::Standard)?,
        None => Chess::default(),
    };
    let mut hist = vec![start];
    if let Some(ms) = &spec.moves {
        for s in ms.split_whitespace() {
            let pos = hist.last().unwrap();
            let m = s.parse::<San>().map_err(|e| anyhow!("{s}: {e}"))?.to_move(pos).map_err(|e| anyhow!("{s}: {e}"))?;
            let mut next = pos.clone();
            next.play_unchecked(m);
            hist.push(next);
        }
    }
    Ok(hist)
}

fn v_star_white(oracle: &dyn Oracle, positions: &[Chess]) -> anyhow::Result<Vec<f64>> {
    let todo: Vec<Chess> = positions.iter().filter(|p| terminal_value(p, Color::White, D).is_none()).cloned().collect();
    let mut res = oracle.engine(&todo, 1)?.into_iter();
    Ok(positions
        .iter()
        .map(|p| match terminal_value(p, Color::White, D) {
            Some(v) => v,
            None => {
                let wdl = res.next().unwrap().lines[0].wdl;
                if p.turn() == Color::White { wdl.score(D) } else { wdl.flip().score(D) }
            }
        })
        .collect())
}

fn trees_t(oracle: &dyn Oracle, hist: &[Chess], k: u32, p: f64, r: Ratings, vw: f64) -> anyhow::Result<(f64, f64, Vec<tree::TreeResult>)> {
    let trees = tree::evaluate(oracle, hist, &[Color::White, Color::Black], TreeParams { k, p, d: D, tau: 1.0, leaf: trickiness_core::leaf::LeafEval::Engine }, r)?;
    Ok((vw - trees[0].value, (1.0 - vw) - trees[1].value, trees))
}

fn sanity(positions: PathBuf, out: PathBuf, ratings: Vec<f32>, ks: Vec<u32>, p: f64, eng: Engines) -> anyhow::Result<()> {
    let specs: Vec<PosSpec> = serde_yaml::from_str(&std::fs::read_to_string(&positions)?)?;
    let oracle = eng.start()?;
    let mut rows = Vec::new();
    for spec in &specs {
        let hist = history_of(spec).with_context(|| spec.id.clone())?;
        let hist = &hist[hist.len().saturating_sub(HISTORY)..];
        let root = hist.last().unwrap();
        let vw = v_star_white(&oracle, std::slice::from_ref(root))?[0];
        for &r in &ratings {
            let rr = Ratings { white: r, black: r };
            let d = oracle.human(&[hist], r, r)?[0].wdl.draw;
            for &k in &ks {
                let (tw, tb, _) = trees_t(&oracle, hist, k, p, rr, vw)?;
                eprintln!("{:32} {r:>6} k={k}  v*={vw:.3}  T_W={tw:+.3}  T_B={tb:+.3}  D={d:.3}", spec.id);
                rows.push(SanityRow {
                    id: spec.id.clone(),
                    category: spec.category.clone(),
                    roles: spec.roles.clone(),
                    fen: full_fen(root),
                    rating: r,
                    k,
                    p,
                    v_star_white: vw,
                    t_white: tw,
                    t_black: tb,
                    draw_rate: d,
                });
            }
        }
    }
    std::fs::write(&out, serde_json::to_string_pretty(&json!({
        "stockfish": oracle.stockfish.name, "nodes": oracle.stockfish.nodes, "maia": oracle.maia.id, "rows": rows
    }))?)?;
    eprintln!("wrote {}", out.display());
    Ok(())
}

fn load_games(path: &PathBuf, limit: Option<usize>) -> anyhow::Result<Vec<Game>> {
    let text = std::fs::read_to_string(path)?;
    let mut games: Vec<Game> = parse_all(&text)?.into_iter().filter_map(|g| g.ok()).collect();
    if let Some(n) = limit {
        games.truncate(n);
    }
    Ok(games)
}

fn done_ids(out: &PathBuf) -> HashSet<String> {
    let Ok(f) = std::fs::File::open(out) else { return HashSet::new() };
    BufReader::new(f)
        .lines()
        .map_while(Result::ok)
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(&l).ok()?["site"].as_str().map(String::from))
        .collect()
}

fn ratings_of(g: &Game) -> Ratings {
    Ratings { white: g.elo("WhiteElo").unwrap_or(1500.0), black: g.elo("BlackElo").unwrap_or(1500.0) }
}

/// (k, p) grid. The largest tree first: the others are subsets of it and come from the cache.
const GRID: [(u32, f64); 6] = [(2, 0.95), (2, 0.9), (2, 0.8), (1, 0.95), (1, 0.9), (1, 0.8)];
/// v* is recorded for this many plies after the sample position (k = 2 needs 2 moves per side).
const PATH: usize = 5;

fn sample(games: PathBuf, out: PathBuf, limit: Option<usize>, eng: Engines) -> anyhow::Result<()> {
    let games = load_games(&games, limit)?;
    let done = done_ids(&out);
    let oracle = eng.start()?;
    let mut f = OpenOptions::new().create(true).append(true).open(&out)?;
    let t0 = Instant::now();
    let todo: Vec<&Game> = games.iter().filter(|g| !done.contains(g.tag("Site").unwrap_or(""))).collect();
    eprintln!("{} games, {} already done, {} to go", games.len(), done.len(), todo.len());
    for (n, g) in todo.iter().enumerate() {
        let t: usize = g.tag("SamplePly").and_then(|s| s.parse().ok()).ok_or_else(|| anyhow!("no SamplePly"))?;
        if t + 1 >= g.positions.len() {
            continue;
        }
        let tg = Instant::now();
        let r = ratings_of(g);
        let hist = &g.positions[(t + 1).saturating_sub(HISTORY)..=t];
        let root = &g.positions[t];
        let end = (t + PATH).min(g.positions.len() - 1);
        let path = v_star_white(&oracle, &g.positions[t..=end])?;
        let vw = path[0];
        let mut tvals = serde_json::Map::new();
        let mut decisions = None;
        for (k, p) in GRID {
            let (tw, tb, trees) = trees_t(&oracle, hist, k, p, r, vw)?;
            tvals.insert(format!("k{k}_p{p}"), json!([tw, tb]));
            if decisions.is_none() {
                decisions = Some(decision_inputs(&oracle, &trees, 5, D)?);
            }
        }
        let mover = root.turn();
        let draw_root = oracle.human(&[hist], r.of(mover), r.of(!mover))?[0].wdl.draw;
        let row = json!({
            "site": g.tag("Site"), "event": g.tag("Event"), "time_control": g.tag("TimeControl"),
            "result": g.tag("Result"), "white_elo": r.white, "black_elo": r.black,
            "plies": g.positions.len() - 1, "t": t, "fen": full_fen(root),
            "side_to_move": if mover == Color::White { "white" } else { "black" },
            "v_star_path_white": path, "T": tvals, "decisions": decisions, "draw_rate_root": draw_root,
            "seconds": tg.elapsed().as_secs_f64(),
        });
        writeln!(f, "{row}")?;
        f.flush()?;
        if n % 10 == 0 {
            let per = t0.elapsed().as_secs_f64() / (n + 1) as f64;
            eprintln!("{}/{}  {:.1}s/game  ~{:.0} min left", n + 1, todo.len(), per, per * (todo.len() - n - 1) as f64 / 60.0);
        }
    }
    Ok(())
}

fn draws(games: PathBuf, out: PathBuf, ply: usize, eng: Engines) -> anyhow::Result<()> {
    let games = load_games(&games, None)?;
    let oracle = eng.start()?;
    let usable: Vec<&Game> = games.iter().filter(|g| g.positions.len() > ply + 1).collect();
    eprintln!("{} of {} games reach ply {ply}", usable.len(), games.len());
    let mut f = std::fs::File::create(&out)?;
    for chunk in usable.chunks(64) {
        let positions: Vec<Chess> = chunk.iter().map(|g| g.positions[ply].clone()).collect();
        let live: Vec<Chess> = positions.iter().filter(|p| !p.is_game_over()).cloned().collect();
        let mut sf = oracle.engine(&live, 1)?.into_iter();
        for (g, pos) in chunk.iter().zip(&positions) {
            if pos.is_game_over() {
                continue;
            }
            let r = ratings_of(g);
            let mover = pos.turn();
            let hist = &g.positions[(ply + 1).saturating_sub(HISTORY)..=ply];
            let maia = oracle.human(&[hist], r.of(mover), r.of(!mover))?.remove(0);
            let wdl = sf.next().unwrap().lines[0].wdl;
            let row = json!({
                "site": g.tag("Site"), "result": g.tag("Result"), "white_elo": r.white, "black_elo": r.black,
                "ply": ply, "maia_wdl": [maia.wdl.win, maia.wdl.draw, maia.wdl.loss],
                "sf_wdl": [wdl.win, wdl.draw, wdl.loss], "mover": if mover == Color::White { "white" } else { "black" },
            });
            writeln!(f, "{row}")?;
        }
        eprint!(".");
    }
    eprintln!("\nwrote {}", out.display());
    Ok(())
}

/// Material on the board (P=1, N=B=3, R=5, Q=9), both sides together: a game-phase proxy.
fn material(pos: &Chess) -> u32 {
    use shakmaty::Role;
    pos.board()
        .clone()
        .into_iter()
        .map(|(_, p)| match p.role {
            Role::Pawn => 1,
            Role::Knight | Role::Bishop => 3,
            Role::Rook => 5,
            Role::Queen => 9,
            Role::King => 0,
        })
        .sum()
}

fn evals(games: PathBuf, out: PathBuf, plies: Vec<usize>, eng: Engines) -> anyhow::Result<()> {
    let games = load_games(&games, None)?;
    let oracle = eng.start()?;
    let mut rows: Vec<(&Game, usize)> = Vec::new();
    for g in &games {
        for &p in &plies {
            if p + 1 < g.positions.len() && !g.positions[p].is_game_over() {
                rows.push((g, p));
            }
        }
    }
    eprintln!("{} positions from {} games", rows.len(), games.len());
    let mut f = std::fs::File::create(&out)?;
    let t0 = Instant::now();
    for (n, chunk) in rows.chunks(64).enumerate() {
        let positions: Vec<Chess> = chunk.iter().map(|(g, p)| g.positions[*p].clone()).collect();
        let res = oracle.engine(&positions, 1)?;
        for ((g, p), r) in chunk.iter().zip(res) {
            let pos = &g.positions[*p];
            let wdl = r.lines[0].wdl;
            let row = json!({
                "cp": r.lines[0].cp, "mate": r.lines[0].mate,
                "site": g.tag("Site"), "result": g.tag("Result"), "time_control": g.tag("TimeControl"),
                "white_elo": g.elo("WhiteElo"), "black_elo": g.elo("BlackElo"), "ply": p,
                "stm": if pos.turn() == Color::White { "white" } else { "black" },
                "sf_wdl": [wdl.win, wdl.draw, wdl.loss], "material": material(pos),
            });
            writeln!(f, "{row}")?;
        }
        if n % 20 == 0 {
            let done = (n + 1) * 64;
            let per = t0.elapsed().as_secs_f64() / done as f64;
            eprintln!("{done}/{}  ~{:.0} min left", rows.len(), per * (rows.len().saturating_sub(done)) as f64 / 60.0);
        }
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().cmd {
        Cmd::Sanity { positions, out, ratings, ks, p, eng } => sanity(positions, out, ratings, ks, p, eng),
        Cmd::Sample { games, out, limit, eng } => sample(games, out, limit, eng),
        Cmd::Draws { games, out, ply, eng } => draws(games, out, ply, eng),
        Cmd::Evals { games, out, plies, eng } => evals(games, out, plies, eng),
    }
}
