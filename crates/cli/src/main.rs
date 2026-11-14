//! `trickiness` command-line tool.
//!
//!   trickiness analyze game.pgn --white-elo 1500 --black-elo 1600 --out game.jsonl
//!   trickiness position "<FEN>" --elo 1500

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{bail, Context};
use clap::{Args, Parser, Subcommand};
use shakmaty::{fen::Fen, CastlingMode, Chess};
use trickiness_core::analysis::{analyze_ply, Settings};
use trickiness_core::engines::EngineOracle;
use trickiness_core::game::{analyze_game, parse_pgn};
use trickiness_core::leaf::LeafKind;
use trickiness_core::practical::{practical, PracticalParams};
use trickiness_core::tree::Ratings;

#[derive(Parser)]
#[command(name = "trickiness", version, about = "Draw rate and per-side trickiness for chess games")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Analyse every ply of a PGN game; writes one JSON object per line as each ply finishes.
    Analyze {
        pgn: PathBuf,
        /// Default: the WhiteElo header, else 1500.
        #[arg(long)]
        white_elo: Option<f32>,
        /// Default: the BlackElo header, else 1500.
        #[arg(long)]
        black_elo: Option<f32>,
        /// Output file (JSON Lines). Default: stdout.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Which game in the PGN, counting from 1.
        #[arg(long, default_value_t = 1)]
        game: usize,
        #[command(flatten)]
        opts: Opts,
    },
    /// Analyse a single position given as FEN.
    Position {
        fen: String,
        /// Rating for both players (overridden by --white-elo / --black-elo).
        #[arg(long, default_value_t = 1500.0)]
        elo: f32,
        #[arg(long)]
        white_elo: Option<f32>,
        #[arg(long)]
        black_elo: Option<f32>,
        #[command(flatten)]
        opts: Opts,
    },
    /// Phase 4: objective best move vs. practical best move against this opponent.
    ///   trickiness practical --moves "e4 e5 Nf3 Nc6 Bc4" --elo 1100
    Practical {
        /// Starting position (default: the normal start). Moves in --moves are played from it.
        #[arg(long)]
        fen: Option<String>,
        /// Moves (SAN, move numbers allowed) leading to the position, so Maia sees the history.
        #[arg(long, default_value = "")]
        moves: String,
        /// Rating for both players (overridden by --white-elo / --black-elo).
        #[arg(long, default_value_t = 1500.0)]
        elo: f32,
        #[arg(long)]
        white_elo: Option<f32>,
        #[arg(long)]
        black_elo: Option<f32>,
        /// Trust in Stockfish's best reply versus the opponent model (1 = engine play).
        #[arg(long, default_value_t = 0.3)]
        lambda: f64,
        /// Temperature for the player's own later moves (below 1 = sharper than Maia).
        #[arg(long, default_value_t = 0.7)]
        tau: f64,
        /// The player's own decisions after the opponent's reply.
        #[arg(long, default_value_t = 1)]
        horizon: u32,
        /// Print only the summary, not the JSON.
        #[arg(long)]
        summary: bool,
        /// Leaf scoring: "calibrated" (human outcome from Stockfish's eval, default), "engine"
        /// (SPEC.md's literal Q) or "maia" (experiment).
        #[arg(long, default_value = "calibrated")]
        leaf: String,
        #[command(flatten)]
        opts: Opts,
    },
}

/// Positions from a start FEN through SAN moves ("1. e4 e5 2. Nf3" works).
fn history_from(fen: Option<&str>, moves: &str) -> anyhow::Result<Vec<Chess>> {
    let start: Chess = match fen {
        Some(f) => f.parse::<Fen>().context("bad FEN")?.into_position(CastlingMode::Standard).context("illegal position")?,
        None => Chess::default(),
    };
    let mut hist = vec![start];
    for tok in moves.split_whitespace() {
        let t = tok.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.');
        if t.is_empty() {
            continue;
        }
        let pos = hist.last().unwrap();
        let m = t
            .parse::<shakmaty::san::San>()
            .map_err(|e| anyhow::anyhow!("{t}: {e}"))?
            .to_move(pos)
            .map_err(|e| anyhow::anyhow!("{t}: {e}"))?;
        let mut next = pos.clone();
        shakmaty::Position::play_unchecked(&mut next, m);
        hist.push(next);
    }
    Ok(hist)
}

#[derive(Args)]
struct Opts {
    /// X's own decisions to look ahead.
    #[arg(long, default_value_t = 2)]
    k: u32,
    /// Share of Maia's probability to expand.
    #[arg(long, default_value_t = 0.9)]
    p: f64,
    /// Stockfish nodes per search.
    #[arg(long, default_value_t = 50_000)]
    nodes: u64,
    /// Value of a draw.
    #[arg(long, default_value_t = 0.5)]
    d: f64,
    /// Stability window (expected score).
    #[arg(long, default_value_t = 0.01)]
    epsilon: f64,
    /// Loss that counts as a trap (expected score).
    #[arg(long, default_value_t = 0.05)]
    delta: f64,
    /// Lines in the stability search.
    #[arg(long, default_value_t = 5)]
    multipv: usize,
    /// Preset: quick (k=1), standard (k=2, 50k nodes), deep (k=2, 200k nodes). Overrides k/p/nodes.
    #[arg(long)]
    preset: Option<String>,
    /// Stockfish processes. Default: physical cores (logical / 2).
    #[arg(long)]
    workers: Option<usize>,
    /// Stockfish hash per process, MB.
    #[arg(long, default_value_t = 16)]
    hash: u32,
    /// Stockfish binary. Default: $TRICKINESS_STOCKFISH, engines/stockfish(.exe), or `stockfish` on PATH.
    #[arg(long)]
    stockfish: Option<PathBuf>,
    /// Maia ONNX model (history-input export). Default: $TRICKINESS_MAIA or models/maia3-5m.history.fp32.onnx.
    #[arg(long)]
    maia_model: Option<PathBuf>,
    /// Syzygy tablebase folder, used if Stockfish finds tables in it. Default: $TRICKINESS_SYZYGY,
    /// else `tablebases/` if it exists. Results record whether tablebases were used.
    #[arg(long)]
    syzygy: Option<PathBuf>,
    /// SQLite cache file.
    #[arg(long, default_value = "trickiness-cache.sqlite")]
    cache: PathBuf,
    /// Don't read or write the on-disk cache.
    #[arg(long)]
    no_cache: bool,
    /// No progress messages on stderr.
    #[arg(long, short)]
    quiet: bool,
}

impl Opts {
    fn apply_preset(&mut self) -> anyhow::Result<()> {
        if let Some(p) = &self.preset {
            let (k, p_, nodes) = match p.to_lowercase().as_str() {
                "quick" => (1, 0.9, 50_000),
                "standard" => (2, 0.9, 50_000),
                "deep" => (2, 0.9, 200_000),
                other => bail!("unknown preset {other} (quick, standard, deep)"),
            };
            self.k = k;
            self.p = p_;
            self.nodes = nodes;
        }
        if self.k == 0 {
            bail!("--k must be at least 1");
        }
        if !(self.p > 0.0 && self.p <= 1.0) {
            bail!("--p must be in (0, 1]");
        }
        Ok(())
    }

    fn stockfish_path(&self) -> PathBuf {
        if let Some(p) = &self.stockfish {
            return p.clone();
        }
        if let Ok(p) = std::env::var("TRICKINESS_STOCKFISH") {
            return p.into();
        }
        let local = Path::new(if cfg!(windows) { "engines/stockfish.exe" } else { "engines/stockfish" });
        if local.exists() {
            return local.to_path_buf();
        }
        PathBuf::from("stockfish")
    }

    fn syzygy_path(&self) -> Option<PathBuf> {
        self.syzygy
            .clone()
            .or_else(|| std::env::var("TRICKINESS_SYZYGY").ok().map(PathBuf::from))
            .or_else(|| Some(PathBuf::from("tablebases")).filter(|p| p.is_dir()))
    }

    fn maia_path(&self) -> PathBuf {
        self.maia_model
            .clone()
            .or_else(|| std::env::var("TRICKINESS_MAIA").ok().map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("models/maia3-5m.history.fp32.onnx"))
    }

    fn build(&self, white_elo: f32, black_elo: f32) -> anyhow::Result<(EngineOracle, Settings)> {
        let workers = self
            .workers
            .unwrap_or_else(|| std::thread::available_parallelism().map(|n| (n.get() / 2).max(1)).unwrap_or(2));
        let oracle = EngineOracle::start(
            &self.stockfish_path(),
            workers,
            self.nodes,
            self.hash,
            self.syzygy_path().as_deref(),
            &self.maia_path(),
            if self.no_cache { None } else { Some(&self.cache) },
        )?;
        let (sf, maia) = (&oracle.stockfish, &oracle.maia);
        let settings = Settings {
            k: self.k,
            p: self.p,
            nodes: self.nodes,
            d: self.d,
            epsilon: self.epsilon,
            delta: self.delta,
            multipv: self.multipv,
            white_elo,
            black_elo,
            stockfish: sf.name.clone(),
            maia: maia.id.clone(),
        };
        if !self.quiet {
            eprintln!(
                "{} × {workers} | {} | k={} p={} nodes={} d={} | White {white_elo} Black {black_elo}",
                sf.name, maia.id, self.k, self.p, self.nodes, self.d
            );
        }
        Ok((oracle, settings))
    }
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().cmd {
        Cmd::Analyze { pgn, white_elo, black_elo, out, game, mut opts } => {
            opts.apply_preset()?;
            if game == 0 {
                bail!("--game counts from 1");
            }
            let text = std::fs::read_to_string(&pgn).with_context(|| format!("reading {}", pgn.display()))?;
            let g = parse_pgn(&text, game - 1)?;
            let we = white_elo.or(g.elo("WhiteElo")).unwrap_or(1500.0);
            let be = black_elo.or(g.elo("BlackElo")).unwrap_or(1500.0);
            let (oracle, settings) = opts.build(we, be)?;
            let mut w: Box<dyn Write> = match &out {
                Some(p) => Box::new(BufWriter::new(File::create(p).with_context(|| format!("creating {}", p.display()))?)),
                None => Box::new(std::io::stdout().lock()),
            };
            let total = g.positions.len();
            let t0 = Instant::now();
            analyze_game(&oracle, &g, &settings, |r| {
                writeln!(w, "{}", serde_json::to_string(r)?)?;
                w.flush()?;
                if !opts.quiet {
                    eprintln!(
                        "ply {:>3}/{} {:<8} v*={:.3} D={:.3} T_W={:+.3} T_B={:+.3}  [{:.0}s]",
                        r.ply,
                        total - 1,
                        r.san.as_deref().unwrap_or("start"),
                        r.v_star,
                        r.draw_rate,
                        r.trickiness.white,
                        r.trickiness.black,
                        t0.elapsed().as_secs_f64()
                    );
                }
                Ok(true)
            })?;
            if !opts.quiet {
                let hits = *oracle.cache.hits.lock().unwrap();
                let misses = *oracle.cache.misses.lock().unwrap();
                eprintln!(
                    "done: {} plies in {:.1}s; cache {hits} hits / {misses} misses",
                    total,
                    t0.elapsed().as_secs_f64()
                );
            }
        }
        Cmd::Position { fen, elo, white_elo, black_elo, mut opts } => {
            opts.apply_preset()?;
            let pos: Chess = fen
                .parse::<Fen>()
                .context("bad FEN")?
                .into_position(CastlingMode::Standard)
                .context("illegal position")?;
            let (oracle, settings) = opts.build(white_elo.unwrap_or(elo), black_elo.unwrap_or(elo))?;
            let r = analyze_ply(&oracle, &[pos], 0, None, &settings)?;
            println!("{}", serde_json::to_string_pretty(&r)?);
        }
        Cmd::Practical { fen, moves, elo, white_elo, black_elo, lambda, tau, horizon, summary, leaf, mut opts } => {
            opts.apply_preset()?;
            let hist = history_from(fen.as_deref(), &moves)?;
            let (we, be) = (white_elo.unwrap_or(elo), black_elo.unwrap_or(elo));
            let (oracle, _) = opts.build(we, be)?;
            let leaf = match leaf.as_str() {
                "calibrated" => LeafKind::Calibrated,
                "engine" => LeafKind::Engine,
                "maia" => LeafKind::Maia,
                other => bail!("--leaf must be calibrated, engine or maia, not {other}"),
            };
            let params = PracticalParams { lambda, tau, d: opts.d, p: opts.p, horizon, delta: opts.delta, leaf, ..PracticalParams::default() };
            let r = practical(&oracle, &hist, Ratings { white: we, black: be }, params)?;
            let ob = &r.candidates[r.objective_best];
            let pb = &r.candidates[r.practical_best];
            let opp = if r.side_to_move == "white" { "Black" } else { "White" };
            eprintln!("Objective best: {} ({:.2}).", ob.san, ob.objective);
            if pb.uci == ob.uci {
                eprintln!("Practical best: the same move ({:.2} against this opponent).", pb.practical);
            } else if !r.clear {
                eprintln!(
                    "Practical best: no clear edge over {} ({} gains only {:+.3}; the margin is {}).",
                    ob.san, pb.san, r.edge, r.params.margin
                );
            } else {
                eprintln!("Practical best: {} ({:.2}, {:+.2} over {} against this opponent).", pb.san, pb.practical, pb.practical - ob.practical, ob.san);
                if let Some(br) = &pb.best_reply {
                    eprintln!(
                        "Relies on: {opp} missing {br}, which Maia says {}s miss {:.0}% of the time. If they find it: {:.2}.",
                        if opp == "White" { we } else { be },
                        100.0 * pb.miss_chance,
                        pb.objective
                    );
                }
            }
            eprintln!("{:<8} {:>9} {:>9} {:>6} {:>7}", "move", "objective", "practical", "miss", "Maia p");
            for c in &r.candidates {
                eprintln!("{:<8} {:>9.3} {:>9.3} {:>5.0}% {:>6.0}%", c.san, c.objective, c.practical, 100.0 * c.miss_chance + 0.0, 100.0 * c.p + 0.0);
            }
            if !summary {
                println!("{}", serde_json::to_string_pretty(&r)?);
            }
        }
    }
    Ok(())
}
