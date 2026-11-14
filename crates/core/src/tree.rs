//! V_X and T_X exactly as SPEC.md defines them. Pure: all engine access goes through
//! [`Oracle`], so the tree can be tested with fakes.
//!
//! The tree is built one level at a time so that every Maia call and every Stockfish search
//! of a level go to the oracle as one batch (batched inference, parallel searches).

use anyhow::{anyhow, Context};
use shakmaty::{uci::UciMove, Chess, Color, KnownOutcome, Outcome, Position};

use crate::leaf::{material, LeafEval};
use crate::oracle::Oracle;

/// Positions of history Maia looks at.
pub const HISTORY: usize = 8;

#[derive(Clone, Copy, Debug)]
pub struct TreeParams {
    /// X's own decisions to look ahead (k >= 1).
    pub k: u32,
    /// Share of Maia's probability to expand at X's nodes.
    pub p: f64,
    /// Value of a draw.
    pub d: f64,
    /// Temperature on Maia at X's nodes: 1.0 = Maia as is (T); below 1 = sharper (Phase 4's
    /// V^τ, X finds the likely good moves more often than Maia says).
    pub tau: f64,
    /// How leaves are scored: Stockfish's W/D/L for T; Phase 4 uses the calibrated model.
    pub leaf: LeafEval,
}

#[derive(Clone, Copy, Debug)]
pub struct Ratings {
    pub white: f32,
    pub black: f32,
}

impl Ratings {
    pub fn of(&self, c: Color) -> f32 {
        match c {
            Color::White => self.white,
            Color::Black => self.black,
        }
    }
}

/// A move Maia proposes at X's first decision node, with its value for X.
#[derive(Clone, Debug)]
pub struct MoveValue {
    pub uci: String,
    /// Maia's raw probability (before renormalisation).
    pub p: f64,
    /// v(position after the move), from X's point of view.
    pub v: f64,
}

#[derive(Clone, Debug)]
pub struct TreeResult {
    pub side: Color,
    /// V_X(root), expected score for X.
    pub value: f64,
    /// X's first decision point: the root if X is to move, otherwise the position after the
    /// opponent's best reply. None if the game ends before X moves.
    pub decision: Option<Decision>,
}

#[derive(Clone, Debug)]
pub struct Decision {
    /// Plies from the root to the decision position: 0 if X is to move at the root, else 1.
    pub offset: u32,
    /// The opponent's move (Stockfish's best) from the root to the decision position, if any.
    pub via: Option<String>,
    /// History ending at the decision position.
    pub history: Vec<Chess>,
    /// The expanded Maia moves, highest probability first.
    pub moves: Vec<MoveValue>,
}

/// Terminal value for `x`, or None if the game isn't over.
pub fn terminal_value(pos: &Chess, x: Color, d: f64) -> Option<f64> {
    match pos.outcome() {
        Outcome::Known(KnownOutcome::Draw) => Some(d),
        Outcome::Known(KnownOutcome::Decisive { winner }) => Some(if winner == x { 1.0 } else { 0.0 }),
        Outcome::Unknown => None,
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Terminal,
    XToMove,
    OppToMove,
}

struct Node {
    hist: Vec<Chess>,
    x: Color,
    /// X decisions still to make.
    dl: u32,
    kind: Kind,
    /// (renormalised weight, child index, uci, raw probability)
    children: Vec<(f64, usize, String, f64)>,
    value: Option<f64>,
    /// Stockfish value of this (opponent-to-move) position for X.
    leaf_v: Option<f64>,
}

fn new_node(nodes: &mut Vec<Node>, hist: Vec<Chess>, x: Color, dl: u32, d: f64) -> usize {
    let pos = hist.last().expect("non-empty history");
    let (kind, value) = match terminal_value(pos, x, d) {
        Some(v) => (Kind::Terminal, Some(v)),
        None if pos.turn() == x => (Kind::XToMove, None),
        None => (Kind::OppToMove, None),
    };
    nodes.push(Node { hist, x, dl, kind, children: Vec::new(), value, leaf_v: None });
    nodes.len() - 1
}

fn play(hist: &[Chess], uci: &str) -> anyhow::Result<Vec<Chess>> {
    let pos = hist.last().unwrap();
    let m = uci
        .parse::<UciMove>()
        .map_err(|e| anyhow!("bad uci {uci}: {e}"))?
        .to_move(pos)
        .with_context(|| format!("illegal move {uci}"))?;
    let mut next = pos.clone();
    next.play_unchecked(m);
    let start = hist.len().saturating_sub(HISTORY - 1);
    let mut h = hist[start..].to_vec();
    h.push(next);
    Ok(h)
}

/// Maia's probabilities at temperature τ: p_i^(1/τ), renormalised. Order is unchanged.
pub fn sharpen(moves: &[(String, f64)], tau: f64) -> Vec<(String, f64)> {
    if (tau - 1.0).abs() < 1e-12 || moves.is_empty() {
        return moves.to_vec();
    }
    let w: Vec<f64> = moves.iter().map(|(_, p)| p.max(0.0).powf(1.0 / tau)).collect();
    let total: f64 = w.iter().sum();
    moves.iter().zip(w).map(|((u, _), x)| (u.clone(), x / total)).collect()
}

/// Moves covering a share `p` of the probability, in order. Always at least one.
pub fn select_moves(moves: &[(String, f64)], p: f64) -> Vec<(String, f64)> {
    let mut out = Vec::new();
    let mut cum = 0.0;
    for (u, pr) in moves {
        out.push((u.clone(), *pr));
        cum += pr;
        if cum >= p - 1e-9 {
            break;
        }
    }
    out
}

/// Evaluate V_X(root) for each side in `sides`. `history` is oldest..root.
pub fn evaluate(
    oracle: &dyn Oracle,
    history: &[Chess],
    sides: &[Color],
    params: TreeParams,
    ratings: Ratings,
) -> anyhow::Result<Vec<TreeResult>> {
    let roots: Vec<(&[Chess], Color)> = sides.iter().map(|&x| (history, x)).collect();
    evaluate_many(oracle, &roots, params, ratings)
}

/// Evaluate V_X for several (history, X) roots at once, batching every level across all
/// of them. Each history is oldest..root.
pub fn evaluate_many(
    oracle: &dyn Oracle,
    roots_in: &[(&[Chess], Color)],
    params: TreeParams,
    ratings: Ratings,
) -> anyhow::Result<Vec<TreeResult>> {
    assert!(params.k >= 1, "k must be at least 1");
    let mut nodes: Vec<Node> = Vec::new();
    let roots: Vec<usize> = roots_in
        .iter()
        .map(|&(history, x)| {
            let start = history.len().saturating_sub(HISTORY);
            new_node(&mut nodes, history[start..].to_vec(), x, params.k, params.d)
        })
        .collect();

    let mut frontier: Vec<usize> = roots.clone();
    while !frontier.is_empty() {
        let mut next = Vec::new();

        // X to move: Maia, batched per side (the ratings differ by side).
        for x in [Color::White, Color::Black] {
            let grp: Vec<usize> =
                frontier.iter().copied().filter(|&i| nodes[i].kind == Kind::XToMove && nodes[i].x == x).collect();
            if grp.is_empty() {
                continue;
            }
            let hists: Vec<&[Chess]> = grp.iter().map(|&i| nodes[i].hist.as_slice()).collect();
            let outs = oracle.human(&hists, ratings.of(x), ratings.of(!x))?;
            for (&i, out) in grp.iter().zip(outs) {
                let raw: std::collections::HashMap<&str, f64> = out.moves.iter().map(|(u, p)| (u.as_str(), *p)).collect();
                let chosen = select_moves(&sharpen(&out.moves, params.tau), params.p);
                let total: f64 = chosen.iter().map(|(_, p)| p).sum();
                if chosen.is_empty() || total <= 0.0 {
                    return Err(anyhow!("Maia returned no moves at a non-terminal position"));
                }
                for (u, pr) in chosen {
                    let h = play(&nodes[i].hist, &u)?;
                    let dl = nodes[i].dl - 1;
                    let c = new_node(&mut nodes, h, x, dl, params.d);
                    // Weight from the (sharpened) distribution; keep Maia's raw p for display.
                    let raw_p = raw.get(u.as_str()).copied().unwrap_or(pr);
                    nodes[i].children.push((pr / total, c, u, raw_p));
                    next.push(c);
                }
            }
        }

        // Leaves scored by Maia (experimental): the opponent is to move, so flip to X's view.
        if params.leaf == LeafEval::Maia {
            for x in [Color::White, Color::Black] {
                let leaves: Vec<usize> = frontier
                    .iter()
                    .copied()
                    .filter(|&i| nodes[i].kind == Kind::OppToMove && nodes[i].dl == 0 && nodes[i].x == x)
                    .collect();
                if leaves.is_empty() {
                    continue;
                }
                let hists: Vec<&[Chess]> = leaves.iter().map(|&i| nodes[i].hist.as_slice()).collect();
                let outs = oracle.human(&hists, ratings.of(!x), ratings.of(x))?;
                for (&i, out) in leaves.iter().zip(outs) {
                    nodes[i].leaf_v = Some(out.wdl.flip().score(params.d));
                }
            }
        }

        // Opponent to move: one Stockfish search gives v(n) and, if needed, the best reply.
        let opps: Vec<usize> = frontier
            .iter()
            .copied()
            .filter(|&i| nodes[i].kind == Kind::OppToMove && !(params.leaf == LeafEval::Maia && nodes[i].dl == 0))
            .collect();
        if !opps.is_empty() {
            let positions: Vec<Chess> = opps.iter().map(|&i| nodes[i].hist.last().unwrap().clone()).collect();
            let res = oracle.engine(&positions, 1)?;
            for (&i, r) in opps.iter().zip(res) {
                let best = r.lines.first().ok_or_else(|| anyhow!("empty engine result"))?;
                // The searched position has the opponent to move, so flip to X's view.
                let x = nodes[i].x;
                let wdl = match params.leaf {
                    LeafEval::Calibrated(m) => {
                        m.wdl(best.pawns(), ratings.of(!x), ratings.of(x), material(nodes[i].hist.last().unwrap()))
                    }
                    _ => best.wdl,
                };
                nodes[i].leaf_v = Some(wdl.flip().score(params.d));
                if nodes[i].dl > 0 {
                    let h = play(&nodes[i].hist, &best.uci)?;
                    let (x, dl) = (nodes[i].x, nodes[i].dl);
                    let c = new_node(&mut nodes, h, x, dl, params.d);
                    nodes[i].children.push((1.0, c, best.uci.clone(), 1.0));
                    next.push(c);
                }
            }
        }
        frontier = next;
    }

    // Children always come after their parent, so a reverse sweep sees children first.
    for i in (0..nodes.len()).rev() {
        if nodes[i].value.is_some() {
            continue;
        }
        let v = match nodes[i].kind {
            Kind::Terminal => unreachable!(),
            Kind::OppToMove if nodes[i].dl == 0 => nodes[i].leaf_v.unwrap(),
            _ => nodes[i].children.iter().map(|&(w, c, _, _)| w * nodes[c].value.unwrap()).sum(),
        };
        nodes[i].value = Some(v);
    }

    Ok(roots
        .iter()
        .map(|&r| {
            let x = nodes[r].x;
            let dec = match nodes[r].kind {
                Kind::XToMove => Some(r),
                Kind::OppToMove => nodes[r]
                    .children
                    .first()
                    .map(|&(_, c, _, _)| c)
                    .filter(|&c| nodes[c].kind == Kind::XToMove),
                Kind::Terminal => None,
            };
            let decision = dec.map(|di| Decision {
                offset: if di == r { 0 } else { 1 },
                via: if di == r { None } else { nodes[r].children.first().map(|c| c.2.clone()) },
                history: nodes[di].hist.clone(),
                moves: nodes[di]
                    .children
                    .iter()
                    .map(|(_, c, u, raw)| MoveValue {
                        uci: u.clone(),
                        p: *raw,
                        v: nodes[*c].leaf_v.or(nodes[*c].value).unwrap(),
                    })
                    .collect(),
            });
            TreeResult { side: x, value: nodes[r].value.unwrap(), decision }
        })
        .collect())
}

#[cfg(test)]
pub(crate) mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use shakmaty::{uci::UciMove, Chess, Color, Position};

    use super::*;
    use crate::oracle::{fen4, EngineLine, EngineResult, HumanResult, Oracle, Wdl};

    /// Answers from hand-written tables keyed by position; panics on anything unexpected, so
    /// a test fails if the tree visits a position it shouldn't.
    #[derive(Default)]
    pub struct Fake {
        pub human: HashMap<String, Vec<(String, f64)>>,
        pub engine: HashMap<String, (String, f64)>, // best move, score for side to move
        pub engine_calls: RefCell<Vec<String>>,
        pub human_calls: RefCell<Vec<(String, f32, f32)>>,
    }

    pub fn pos(moves: &str) -> Chess {
        let mut p = Chess::default();
        for u in moves.split_whitespace() {
            let m = u.parse::<UciMove>().unwrap().to_move(&p).unwrap();
            p.play_unchecked(m);
        }
        p
    }

    pub fn hist(moves: &str) -> Vec<Chess> {
        let mut out = vec![Chess::default()];
        for u in moves.split_whitespace() {
            let mut p = out.last().unwrap().clone();
            p.play_unchecked(u.parse::<UciMove>().unwrap().to_move(&p).unwrap());
            out.push(p);
        }
        out
    }

    /// A WDL with no wins that scores exactly `s` for the side to move (d = 0.5) when s <= 0.5,
    /// else no losses.
    pub fn wdl_for(s: f64) -> Wdl {
        if s <= 0.5 {
            Wdl::new(0.0, 2.0 * s, 1.0 - 2.0 * s)
        } else {
            Wdl::new(2.0 * s - 1.0, 2.0 - 2.0 * s, 0.0)
        }
    }

    impl Fake {
        pub fn h(mut self, moves: &str, probs: &[(&str, f64)]) -> Self {
            self.human.insert(fen4(&pos(moves)), probs.iter().map(|(u, p)| (u.to_string(), *p)).collect());
            self
        }
        /// `score` is for the side to move at that position.
        pub fn e(mut self, moves: &str, best: &str, score: f64) -> Self {
            self.engine.insert(fen4(&pos(moves)), (best.to_string(), score));
            self
        }
    }

    impl Oracle for Fake {
        fn engine(&self, positions: &[Chess], _multipv: usize) -> anyhow::Result<Vec<EngineResult>> {
            Ok(positions
                .iter()
                .map(|p| {
                    let k = fen4(p);
                    self.engine_calls.borrow_mut().push(k.clone());
                    let (best, s) = self.engine.get(&k).unwrap_or_else(|| panic!("no engine entry for {k}"));
                    EngineResult { lines: vec![EngineLine { uci: best.clone(), wdl: wdl_for(*s), depth: 1, cp: None, mate: None }] }
                })
                .collect())
        }
        fn human(&self, histories: &[&[Chess]], es: f32, eo: f32) -> anyhow::Result<Vec<HumanResult>> {
            Ok(histories
                .iter()
                .map(|h| {
                    let k = fen4(h.last().unwrap());
                    self.human_calls.borrow_mut().push((k.clone(), es, eo));
                    let moves = self.human.get(&k).unwrap_or_else(|| panic!("no human entry for {k}")).clone();
                    HumanResult { moves, wdl: Wdl::new(0.0, 1.0, 0.0) }
                })
                .collect())
        }
    }

    const P: TreeParams = TreeParams { k: 1, p: 1.0, d: 0.5, tau: 1.0, leaf: LeafEval::Engine };
    const R: Ratings = Ratings { white: 1500.0, black: 1500.0 };

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "{a} != {b}");
    }

    /// SPEC.md's example: a move worth 0.45 objectively, where the opponent errs 40% of the
    /// time into 0.85, is worth 0.61. Here Black has just played that move; White is the
    /// opponent who errs (X = White, k = 1). After 1.e4 Black is to move, scores from Black.
    #[test]
    fn spec_example_0_61() {
        let f = Fake::default()
            .h("", &[("e2e4", 0.6), ("d2d4", 0.4)])
            .e("e2e4", "e7e5", 0.45) // White's good move: Black scores 0.45
            .e("d2d4", "e7e5", 0.85); // White's error: Black scores 0.85
        let r = evaluate(&f, &hist(""), &[Color::White], P, R).unwrap();
        close(r[0].value, 0.6 * 0.55 + 0.4 * 0.15); // V_White = 0.39
        close(1.0 - r[0].value, 0.61); // worth 0.61 to Black
    }

    /// Same example with the opponent (Black) to move at the root: Black plays Stockfish's
    /// best move, then White decides once.
    #[test]
    fn opponent_to_move_at_root_plays_engine_best() {
        let f = Fake::default()
            .e("e2e4", "e7e5", 0.45)
            .h("e2e4 e7e5", &[("g1f3", 0.6), ("f1c4", 0.4)])
            .e("e2e4 e7e5 g1f3", "b8c6", 0.45)
            .e("e2e4 e7e5 f1c4", "b8c6", 0.85);
        let r = evaluate(&f, &hist("e2e4"), &[Color::White], P, R).unwrap();
        close(r[0].value, 0.39);
        let dec = r[0].decision.as_ref().unwrap();
        assert_eq!(fen4(dec.history.last().unwrap()), fen4(&pos("e2e4 e7e5")));
        assert_eq!(dec.moves.len(), 2);
        close(dec.moves[0].v, 0.55);
        close(dec.moves[1].v, 0.15);
    }

    /// p cutoff: 0.5 + 0.3 reaches p = 0.8, so only two moves are expanded and renormalised
    /// to 0.625 / 0.375. The third move's position is never searched (the fake would panic).
    #[test]
    fn p_cutoff_renormalises() {
        let f = Fake::default()
            .h("", &[("e2e4", 0.5), ("d2d4", 0.3), ("g2g4", 0.15), ("f2f3", 0.05)])
            .e("e2e4", "e7e5", 0.4)
            .e("d2d4", "d7d5", 0.6);
        let p = TreeParams { p: 0.8, ..P };
        let r = evaluate(&f, &hist(""), &[Color::White], p, R).unwrap();
        close(r[0].value, 0.625 * 0.6 + 0.375 * 0.4);
        assert_eq!(f.engine_calls.borrow().len(), 2);
    }

    /// k = 2 by hand. White (X) moves, Black replies with Stockfish's move, White moves again.
    ///   1.e4 (0.7): ...e5, then 2.Nf3 (0.5) -> W 0.6, 2.Qh5 (0.5) -> W 0.2  => 0.4
    ///   1.d4 (0.3): ...d5, then 2.c4 (1.0)  -> W 0.5                        => 0.5
    ///   V = 0.7*0.4 + 0.3*0.5 = 0.43
    #[test]
    fn k2_hand_built() {
        let f = Fake::default()
            .h("", &[("e2e4", 0.7), ("d2d4", 0.3)])
            .e("e2e4", "e7e5", 0.5)
            .e("d2d4", "d7d5", 0.5)
            .h("e2e4 e7e5", &[("g1f3", 0.5), ("d1h5", 0.5)])
            .h("d2d4 d7d5", &[("c2c4", 1.0)])
            .e("e2e4 e7e5 g1f3", "b8c6", 0.4)
            .e("e2e4 e7e5 d1h5", "b8c6", 0.8)
            .e("d2d4 d7d5 c2c4", "e7e6", 0.5);
        let p = TreeParams { k: 2, ..P };
        let r = evaluate(&f, &hist(""), &[Color::White], p, R).unwrap();
        close(r[0].value, 0.43);
        // The root's decision moves report the one-move values v(n·m), not V.
        let dec = r[0].decision.as_ref().unwrap();
        close(dec.moves[0].v, 0.5);
    }

    /// Maia is asked with X's rating as "self" and the opponent's as "oppo".
    #[test]
    fn ratings_passed_per_side() {
        let f = Fake::default().h("e2e4", &[("e7e5", 1.0)]).e("e2e4 e7e5", "g1f3", 0.5);
        let r = Ratings { white: 1100.0, black: 1900.0 };
        evaluate(&f, &hist("e2e4"), &[Color::Black], P, r).unwrap();
        assert_eq!(f.human_calls.borrow()[0].1, 1900.0);
        assert_eq!(f.human_calls.borrow()[0].2, 1100.0);
    }

    /// Mates and stalemates are scored without engines: after 1.f3 e5 2.g4, Black's Qh4# is 1.
    #[test]
    fn terminal_positions() {
        let f = Fake::default()
            .h("f2f3 e7e5 g2g4", &[("d8h4", 0.5), ("b8c6", 0.5)])
            .e("f2f3 e7e5 g2g4 b8c6", "f1g2", 0.4); // White to move scores 0.4 -> Black 0.6
        let r = evaluate(&f, &hist("f2f3 e7e5 g2g4"), &[Color::Black], P, R).unwrap();
        close(r[0].value, 0.5 * 1.0 + 0.5 * 0.6);
        // A mated root has no decision and the terminal value.
        let r = evaluate(&f, &hist("f2f3 e7e5 g2g4 d8h4"), &[Color::White, Color::Black], P, R).unwrap();
        close(r[0].value, 0.0);
        close(r[1].value, 1.0);
        assert!(r[0].decision.is_none());
    }

    #[test]
    fn draw_value_d_is_used() {
        // Stalemate: Black king a8, White queen b6, Black to move.
        let fen: shakmaty::fen::Fen = "k7/8/1Q6/8/8/8/8/2K5 b - - 0 1".parse().unwrap();
        let st: Chess = fen.into_position(shakmaty::CastlingMode::Standard).unwrap();
        assert_eq!(terminal_value(&st, Color::White, 0.3), Some(0.3));
        assert_eq!(terminal_value(&st, Color::Black, 0.3), Some(0.3));
    }

    /// τ = 0.5 squares and renormalises: 0.6/0.4 -> 0.36/0.16 -> 0.692/0.308.
    #[test]
    fn temperature_sharpens_maia() {
        let f = Fake::default()
            .h("", &[("e2e4", 0.6), ("d2d4", 0.4)])
            .e("e2e4", "e7e5", 0.45)
            .e("d2d4", "e7e5", 0.85);
        let r = evaluate(&f, &hist(""), &[Color::White], TreeParams { tau: 0.5, ..P }, R).unwrap();
        let (a, b) = (0.36 / 0.52, 0.16 / 0.52);
        close(r[0].value, a * 0.55 + b * 0.15);
        // The decision still reports Maia's raw probabilities.
        close(r[0].decision.as_ref().unwrap().moves[0].p, 0.6);
        let s = sharpen(&[("a".into(), 0.6), ("b".into(), 0.4)], 1.0);
        close(s[0].1, 0.6);
    }

    /// Several roots in one call give the same values as separate calls.
    #[test]
    fn evaluate_many_matches_single_roots() {
        let f = Fake::default()
            .h("", &[("e2e4", 0.6), ("d2d4", 0.4)])
            .e("", "e2e4", 0.5)
            .e("e2e4", "e7e5", 0.45)
            .e("d2d4", "e7e5", 0.85)
            .h("e2e4", &[("e7e5", 1.0)])
            .e("e2e4 e7e5", "g1f3", 0.55);
        let a = hist("");
        let b = hist("e2e4");
        let many = evaluate_many(&f, &[(&a, Color::White), (&b, Color::Black)], P, R).unwrap();
        close(many[0].value, evaluate(&f, &a, &[Color::White], P, R).unwrap()[0].value);
        close(many[1].value, evaluate(&f, &b, &[Color::Black], P, R).unwrap()[0].value);
    }

    /// With human leaves, leaf values come from Maia's value head (the fake predicts a sure
    /// draw), so V = d whatever Stockfish says, and no engine search is made at the leaves.
    #[test]
    fn human_leaves_use_maia_values() {
        let f = Fake::default()
            .h("", &[("e2e4", 0.6), ("d2d4", 0.4)])
            .h("e2e4", &[("e7e5", 1.0)])
            .h("d2d4", &[("d7d5", 1.0)]);
        let r = evaluate(&f, &hist(""), &[Color::White], TreeParams { leaf: LeafEval::Maia, d: 0.3, ..P }, R).unwrap();
        close(r[0].value, 0.3);
        assert!(f.engine_calls.borrow().is_empty());
    }

    #[test]
    fn select_moves_always_takes_one() {
        let m = vec![("a".to_string(), 0.95), ("b".to_string(), 0.05)];
        assert_eq!(select_moves(&m, 0.9).len(), 1);
        assert_eq!(select_moves(&m, 0.99).len(), 2);
    }
}
