//! Per-ply metrics: v*, D, T_White, T_Black and the diagnostics. Pure, like the tree: all
//! engine access goes through [`Oracle`].

use serde::{Deserialize, Serialize};
use shakmaty::{san::SanPlus, uci::UciMove, Chess, Color, Position};

use crate::oracle::{full_fen, turn_name, Oracle};
use crate::tree::{self, terminal_value, Ratings, TreeParams};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    pub k: u32,
    pub p: f64,
    pub nodes: u64,
    pub d: f64,
    pub epsilon: f64,
    pub delta: f64,
    pub multipv: usize,
    pub white_elo: f32,
    pub black_elo: f32,
    /// Engine and model identities, so every number can be traced.
    pub stockfish: String,
    pub maia: String,
}

impl Settings {
    pub fn ratings(&self) -> Ratings {
        Ratings { white: self.white_elo, black: self.black_elo }
    }
    pub fn tree_params(&self) -> TreeParams {
        TreeParams { k: self.k, p: self.p, d: self.d, tau: 1.0, leaf: crate::leaf::LeafEval::Engine }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sides<T> {
    pub white: T,
    pub black: T,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Diagnostics {
    /// Moves within ε of the best, from a Stockfish MultiPV search (capped at `multipv`).
    pub stable_moves: u32,
    /// Maia probability on expanded moves that lose more than δ against Stockfish's best.
    pub trap_mass: f64,
    /// Plies from this position to the player's decision point (0 if they're to move).
    pub at_ply_offset: u32,
    /// The decision position (this ply's position, or after the opponent's best reply).
    pub decision_fen: String,
    /// The opponent's best reply leading to the decision position, in SAN (offset 1 only).
    pub via: Option<String>,
    /// v after Stockfish's best move at the decision position, for this player.
    pub best_value: f64,
    /// Maia's likely moves at the decision position (the expanded top-p set), with values.
    pub moves: Vec<LikelyMove>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LikelyMove {
    pub san: String,
    pub uci: String,
    /// Maia's probability at the mover's rating.
    pub p: f64,
    /// v after the move, for the mover.
    pub v: f64,
    /// True if the move loses more than δ against Stockfish's best move.
    pub loses: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlyResult {
    /// Half-moves played to reach this position (0 = start).
    pub ply: u32,
    /// The move that led here.
    pub san: Option<String>,
    pub uci: Option<String>,
    pub fen: String,
    pub side_to_move: String,
    /// Stockfish's expected score for White.
    pub v_star: f64,
    /// Maia's draw probability at (mover's rating, other rating).
    pub draw_rate: f64,
    /// T for each player, from that player's point of view. Raw: may be slightly negative.
    pub trickiness: Sides<f64>,
    pub diagnostics: Sides<Option<Diagnostics>>,
    /// Maia's likely moves for the side to move, with values. Empty if the game is over.
    pub likely_moves: Vec<LikelyMove>,
    pub settings: Settings,
}

mod color_name {
    use serde::{Deserialize, Deserializer, Serializer};
    use shakmaty::Color;

    pub fn serialize<S: Serializer>(c: &Color, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(if *c == Color::White { "white" } else { "black" })
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Color, D::Error> {
        Ok(if String::deserialize(d)? == "white" { Color::White } else { Color::Black })
    }
}

fn r4(x: f64) -> f64 {
    // `+ 0.0` turns -0.0 (e.g. an empty f64 sum) into 0.0.
    (x * 10_000.0).round() / 10_000.0 + 0.0
}

/// Stability and trap mass from the values already in hand.
/// `mpv_scores`: MultiPV line scores for X (X to move). `v_best`: v after Stockfish's best
/// move, for X. `moves`: (Maia probability, v for X) of the expanded moves.
pub fn diagnostics(mpv_scores: &[f64], v_best: f64, moves: &[(f64, f64)], eps: f64, delta: f64) -> (u32, f64) {
    let top = mpv_scores.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let stable = mpv_scores.iter().filter(|&&s| s >= top - eps).count() as u32;
    let trap = moves.iter().filter(|(_, v)| *v < v_best - delta).map(|(p, _)| p).sum();
    (stable, trap)
}

fn play_uci(pos: &Chess, uci: &str) -> anyhow::Result<(Chess, shakmaty::Move)> {
    let m = uci.parse::<UciMove>().map_err(|e| anyhow::anyhow!("{e}"))?.to_move(pos)?;
    let mut p = pos.clone();
    p.play_unchecked(m.clone());
    Ok((p, m))
}

/// What the diagnostics are computed from, at one player's decision point. Kept separate so
/// ε and δ can be re-tuned without new engine calls (Phase 2).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DecisionInputs {
    #[serde(with = "color_name")]
    pub side: Color,
    pub offset: u32,
    /// MultiPV line scores for the player (who is to move there).
    pub mpv_scores: Vec<f64>,
    /// v after Stockfish's best move, for the player, from a MultiPV-1 search like the moves'.
    pub v_best: f64,
    /// (Maia probability, v for the player) of the expanded moves.
    pub moves: Vec<(f64, f64)>,
    /// UCI of each expanded move, same order as `moves`.
    #[serde(default)]
    pub ucis: Vec<String>,
    /// FEN of the decision position.
    #[serde(default)]
    pub fen: String,
    /// The opponent's move leading to it (UCI), if any.
    #[serde(default)]
    pub via: Option<String>,
}

/// Diagnostic inputs at each tree's decision point: one MultiPV search there, plus the value
/// after Stockfish's best move measured the same way as the Maia moves' values.
pub fn decision_inputs(
    oracle: &dyn Oracle,
    trees: &[tree::TreeResult],
    multipv: usize,
    d: f64,
) -> anyhow::Result<Vec<DecisionInputs>> {
    let decisions: Vec<_> = trees.iter().filter_map(|t| t.decision.as_ref().map(|dd| (t.side, dd))).collect();
    let dpos: Vec<Chess> = decisions.iter().map(|(_, dd)| dd.history.last().unwrap().clone()).collect();
    let mpv = oracle.engine(&dpos, multipv)?;
    let mut best_children = Vec::new();
    let mut best_values: Vec<Option<f64>> = Vec::new();
    for ((x, _), (pos, r)) in decisions.iter().zip(dpos.iter().zip(&mpv)) {
        let (child, _) = play_uci(pos, &r.lines[0].uci)?;
        match terminal_value(&child, *x, d) {
            Some(v) => best_values.push(Some(v)),
            None => {
                best_values.push(None);
                best_children.push(child);
            }
        }
    }
    let mut child_res = oracle.engine(&best_children, 1)?.into_iter();
    let mut out = Vec::new();
    for (i, (x, dec)) in decisions.iter().enumerate() {
        let v_best = match best_values[i] {
            Some(v) => v,
            None => child_res.next().unwrap().lines[0].wdl.flip().score(d),
        };
        out.push(DecisionInputs {
            side: *x,
            offset: dec.offset,
            mpv_scores: mpv[i].lines.iter().map(|l| l.wdl.score(d)).collect(),
            v_best,
            moves: dec.moves.iter().map(|m| (m.p, m.v)).collect(),
            ucis: dec.moves.iter().map(|m| m.uci.clone()).collect(),
            fen: full_fen(dec.history.last().unwrap()),
            via: dec.via.clone(),
        });
    }
    Ok(out)
}

/// Analyse the last position of `history` (oldest..current).
pub fn analyze_ply(
    oracle: &dyn Oracle,
    history: &[Chess],
    ply: u32,
    last_move: Option<(String, String)>,
    s: &Settings,
) -> anyhow::Result<PlyResult> {
    let root = history.last().expect("non-empty history");
    let mover = root.turn();
    let ratings = s.ratings();
    let (san, uci) = match last_move {
        Some((a, b)) => (Some(a), Some(b)),
        None => (None, None),
    };

    let maia = oracle.human(&[history], ratings.of(mover), ratings.of(!mover))?.remove(0);
    let mut out = PlyResult {
        ply,
        san,
        uci,
        fen: full_fen(root),
        side_to_move: turn_name(root).to_string(),
        v_star: 0.0,
        draw_rate: r4(maia.wdl.draw),
        trickiness: Sides { white: 0.0, black: 0.0 },
        diagnostics: Sides { white: None, black: None },
        likely_moves: Vec::new(),
        settings: s.clone(),
    };

    if let Some(v) = terminal_value(root, Color::White, s.d) {
        out.v_star = r4(v);
        return Ok(out);
    }

    // v*: a MultiPV-1 search, the same kind as the tree's leaves (DECISIONS 0.11).
    let root_wdl = oracle.engine(std::slice::from_ref(root), 1)?[0].lines[0].wdl;
    let wdl_white = if mover == Color::White { root_wdl } else { root_wdl.flip() };
    let v_star = |c: Color| if c == Color::White { wdl_white.score(s.d) } else { wdl_white.flip().score(s.d) };
    out.v_star = r4(v_star(Color::White));

    let trees = tree::evaluate(oracle, history, &[Color::White, Color::Black], s.tree_params(), ratings)?;
    out.trickiness = Sides {
        white: r4(v_star(Color::White) - trees[0].value),
        black: r4(v_star(Color::Black) - trees[1].value),
    };

    let decision_list = decision_inputs(oracle, &trees, s.multipv, s.d)?;
    for (raw, dec) in decision_list.iter().zip(trees.iter().filter_map(|t| t.decision.as_ref())) {
        let (stable, trap) = diagnostics(&raw.mpv_scores, raw.v_best, &raw.moves, s.epsilon, s.delta);
        let dpos = dec.history.last().unwrap();
        let mut moves = Vec::new();
        for m in &dec.moves {
            let (_, mv) = play_uci(dpos, &m.uci)?;
            moves.push(LikelyMove {
                san: SanPlus::from_move(dpos.clone(), mv).to_string(),
                uci: m.uci.clone(),
                p: r4(m.p),
                v: r4(m.v),
                loses: m.v < raw.v_best - s.delta,
            });
        }
        let via = match &dec.via {
            Some(u) => {
                let (_, mv) = play_uci(root, u)?;
                Some(SanPlus::from_move(root.clone(), mv).to_string())
            }
            None => None,
        };
        if raw.side == mover {
            // Likely moves for the side to move = its tree's root decision (spec's field).
            out.likely_moves = moves.clone();
        }
        let d = Diagnostics {
            stable_moves: stable,
            trap_mass: r4(trap),
            at_ply_offset: raw.offset,
            decision_fen: raw.fen.clone(),
            via,
            best_value: r4(raw.v_best),
            moves,
        };
        match raw.side {
            Color::White => out.diagnostics.white = Some(d),
            Color::Black => out.diagnostics.black = Some(d),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::tests::{hist, Fake};

    #[test]
    fn diagnostics_by_hand() {
        // MultiPV scores 0.60, 0.58, 0.55, 0.40 with ε = 0.03 -> 2 stable moves.
        // Maia: 0.5 on a move worth 0.59, 0.3 on 0.45, 0.2 on 0.20; best 0.60, δ = 0.10
        // -> trap mass 0.3 + 0.2.
        let (st, tr) = diagnostics(&[0.60, 0.58, 0.55, 0.40], 0.60, &[(0.5, 0.59), (0.3, 0.45), (0.2, 0.20)], 0.03, 0.10);
        assert_eq!(st, 2);
        assert!((tr - 0.5).abs() < 1e-12);
    }

    /// End to end through analyze_ply with the fake: the spec's 0.61 position.
    #[test]
    fn analyze_ply_spec_example() {
        let f = Fake::default()
            .h("", &[("e2e4", 0.6), ("d2d4", 0.4)])
            .e("", "e2e4", 0.55) // root: White scores 0.55
            .e("e2e4", "e7e5", 0.45)
            .e("d2d4", "e7e5", 0.85)
            // Black's tree: White (opponent) plays e4, then Black decides.
            .h("e2e4", &[("e7e5", 1.0)])
            .e("e2e4 e7e5", "g1f3", 0.55);
        let s = Settings {
            k: 1, p: 1.0, nodes: 1, d: 0.5, epsilon: 0.03, delta: 0.1, multipv: 1,
            white_elo: 1500.0, black_elo: 1500.0, stockfish: "fake".into(), maia: "fake".into(),
        };
        let r = analyze_ply(&f, &hist(""), 0, None, &s).unwrap();
        assert_eq!(r.v_star, 0.55);
        assert_eq!(r.trickiness.white, 0.16); // 0.55 - 0.39
        assert_eq!(r.trickiness.black, 0.0); // 0.45 - 0.45
        let w = r.diagnostics.white.unwrap();
        assert_eq!(w.stable_moves, 1);
        assert_eq!(w.trap_mass, 0.4); // d4 drops from 0.55 to 0.15
        assert_eq!(r.likely_moves[0].san, "e4");
        assert_eq!(r.likely_moves[1].v, 0.15);
        // d4 drops 0.40 against the best move (> δ = 0.1); e4 is the best move.
        assert!(r.likely_moves[1].loses && !r.likely_moves[0].loses);
        assert_eq!(w.best_value, 0.55);
        assert!(w.via.is_none());
        // Black decides after White's best reply, 1.e4.
        let b = r.diagnostics.black.unwrap();
        assert_eq!(b.at_ply_offset, 1);
        assert_eq!(b.via.as_deref(), Some("e4"));
        assert_eq!(b.moves[0].san, "e5");
        assert!(b.decision_fen.starts_with("rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b"));
    }
}
