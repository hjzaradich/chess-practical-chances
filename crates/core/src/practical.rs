//! Phase 4: the practical value of a move against this particular opponent (SPEC.md).
//!
//!   Q(m) = Σ_r [(1-λ) P_opp(r) + λ·1{r = r*}] · V^τ_X(n·m·r)
//!
//! Candidates: Stockfish's top moves plus Maia's top moves for X. The opponent's replies are
//! Maia at their rating (top-p, renormalised) blended with Stockfish's best reply r*. After
//! the reply, X's own later decisions use Maia at X's rating sharpened by τ < 1, scored as in
//! the T tree (opponent plays Stockfish's best; leaves are Stockfish values with draw = d).
//! Pure: all engine access goes through [`Oracle`].

use anyhow::{anyhow, Context};
use serde::{Deserialize, Serialize};
use shakmaty::{san::SanPlus, uci::UciMove, Chess, Color, Position};

use crate::leaf::{material, LeafEval, LeafKind};
use crate::oracle::{full_fen, turn_name, Oracle};
use crate::tree::{self, select_moves, terminal_value, Ratings, TreeParams, HISTORY};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct PracticalParams {
    /// Trust in Stockfish's best reply versus the opponent model (λ = 1: engine play).
    pub lambda: f64,
    /// Temperature for X's own later moves (below 1 = sharper than Maia).
    pub tau: f64,
    /// Value of a draw (0.5 normally, 0 in a must-win game).
    pub d: f64,
    /// Share of Maia's probability expanded at each human node.
    pub p: f64,
    /// X's own decisions after the reply (0 = score right after the reply).
    pub horizon: u32,
    /// A reply that gives X more than δ over the best reply counts as a miss.
    pub delta: f64,
    /// Candidates: this many Stockfish moves plus this many Maia moves (5 + 8, DECISIONS 4.6).
    pub n_engine: usize,
    pub n_human: usize,
    /// How leaves are scored (DECISIONS 4.5): "calibrated" (default), "engine" (SPEC.md's
    /// literal Q) or "maia" (experiment, 4.3).
    #[serde(default = "default_leaf")]
    pub leaf: LeafKind,
    /// A practical best is shown only if it beats the objective best by at least this much
    /// (SPEC.md Phase 4, DECISIONS 4.8).
    #[serde(default = "default_margin")]
    pub margin: f64,
}

fn default_margin() -> f64 {
    0.02
}

fn default_leaf() -> LeafKind {
    LeafKind::Calibrated
}

impl Default for PracticalParams {
    fn default() -> Self {
        PracticalParams { lambda: 0.3, tau: 0.7, d: 0.5, p: 0.9, horizon: 1, delta: 0.05, n_engine: 5, n_human: 8, leaf: LeafKind::Calibrated, margin: 0.02 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Reply {
    pub san: String,
    pub uci: String,
    /// Maia's probability that the opponent plays it (raw).
    pub p: f64,
    /// Weight in Q: (1-λ)·P̃ + λ for Stockfish's best reply.
    pub weight: f64,
    /// Stockfish's value for X after the reply.
    pub objective: f64,
    /// V^τ_X after the reply: X's practical value from there.
    pub value: f64,
    pub is_best: bool,
    /// Gives X more than δ over the best reply.
    pub miss: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Candidate {
    pub san: String,
    pub uci: String,
    /// Maia's probability that X plays it at X's rating (raw).
    pub p: f64,
    /// 1-based rank in Stockfish's MultiPV, if it's one of its top moves.
    pub engine_rank: Option<usize>,
    /// Stockfish's value of the move for X (the opponent replies perfectly).
    pub objective: f64,
    /// Q(m): its value against this opponent.
    pub practical: f64,
    /// Stockfish's best reply, if the game goes on.
    pub best_reply: Option<String>,
    /// Maia's chance that the opponent misses the best reply (plays a reply > δ worse for them).
    pub miss_chance: f64,
    pub replies: Vec<Reply>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Practical {
    pub fen: String,
    pub side_to_move: String,
    /// Sorted by practical value, best first.
    pub candidates: Vec<Candidate>,
    /// Index into `candidates` of Stockfish's preferred move (highest objective value).
    pub objective_best: usize,
    /// Index of the move with the highest practical value (0, since sorted).
    pub practical_best: usize,
    /// Q(practical best) − Q(objective best): what the practical choice gains against this
    /// opponent over the engine's move (0 when they're the same move).
    pub edge: f64,
    /// The practical best is a different move and its edge clears `params.margin`. When false,
    /// the display says there's no practical edge over the engine move.
    pub clear: bool,
    pub params: PracticalParams,
    pub white_elo: f32,
    pub black_elo: f32,
    /// Name of the calibrated leaf model, if used (traceability).
    pub leaf_model: Option<String>,
}

fn play(hist: &[Chess], uci: &str) -> anyhow::Result<(Vec<Chess>, String)> {
    let pos = hist.last().unwrap();
    let m = uci.parse::<UciMove>().map_err(|e| anyhow!("bad uci {uci}: {e}"))?.to_move(pos).with_context(|| format!("illegal {uci}"))?;
    let san = SanPlus::from_move(pos.clone(), m.clone()).to_string();
    let mut next = pos.clone();
    next.play_unchecked(m);
    let mut h = hist[hist.len().saturating_sub(HISTORY - 1)..].to_vec();
    h.push(next);
    Ok((h, san))
}

/// Value for `x` of a position from a MultiPV-1 search there (or its terminal value).
fn score_for(x: Color, pos: &Chess, wdl: crate::oracle::Wdl, d: f64) -> f64 {
    if pos.turn() == x { wdl.score(d) } else { wdl.flip().score(d) }
}

/// Practical values of the candidate moves at the last position of `history`.
pub fn practical(
    oracle: &dyn Oracle,
    history: &[Chess],
    ratings: Ratings,
    params: PracticalParams,
) -> anyhow::Result<Practical> {
    let hist = &history[history.len().saturating_sub(HISTORY)..];
    let root = hist.last().ok_or_else(|| anyhow!("empty history"))?;
    if root.is_game_over() {
        return Err(anyhow!("the game is over in this position"));
    }
    let x = root.turn();
    let (rx, ro) = (ratings.of(x), ratings.of(!x));

    // 1. Candidates: Stockfish's top moves, then Maia's top moves not already listed.
    let lines = oracle.engine(std::slice::from_ref(root), params.n_engine)?.remove(0).lines;
    let maia_root = oracle.human(&[hist], rx, ro)?.remove(0);
    let mut cands: Vec<(String, Option<usize>)> = lines.iter().enumerate().map(|(i, l)| (l.uci.clone(), Some(i + 1))).collect();
    for (u, _) in maia_root.moves.iter().take(params.n_human) {
        if !cands.iter().any(|(c, _)| c == u) {
            cands.push((u.clone(), None));
        }
    }
    let maia_p = |u: &str| maia_root.moves.iter().find(|(m, _)| m == u).map(|(_, p)| *p).unwrap_or(0.0);

    // 2. After each candidate: Stockfish (objective value + best reply) and Maia's replies.
    let mut after: Vec<(Vec<Chess>, String)> = Vec::new();
    for (u, _) in &cands {
        after.push(play(hist, u)?);
    }
    let live: Vec<usize> = (0..after.len()).filter(|&i| terminal_value(after[i].0.last().unwrap(), x, params.d).is_none()).collect();
    let live_pos: Vec<Chess> = live.iter().map(|&i| after[i].0.last().unwrap().clone()).collect();
    let sf_after = oracle.engine(&live_pos, 1)?;
    let live_hist: Vec<&[Chess]> = live.iter().map(|&i| after[i].0.as_slice()).collect();
    let human_after = oracle.human(&live_hist, ro, rx)?;

    // 3. Reply sets and the positions after each reply.
    struct Pending {
        cand: usize,
        uci: String,
        san: String,
        p: f64,
        weight: f64,
        is_best: bool,
        hist: Vec<Chess>,
    }
    let mut pend: Vec<Pending> = Vec::new();
    let mut best_reply: Vec<Option<String>> = vec![None; cands.len()];
    for (j, &i) in live.iter().enumerate() {
        let best = sf_after[j].lines.first().ok_or_else(|| anyhow!("empty engine result"))?.uci.clone();
        let chosen = select_moves(&human_after[j].moves, params.p);
        let total: f64 = chosen.iter().map(|(_, p)| p).sum();
        let mut set: Vec<(String, f64, f64)> = chosen // (uci, raw p, weight)
            .iter()
            .map(|(u, p)| (u.clone(), *p, (1.0 - params.lambda) * p / total))
            .collect();
        match set.iter_mut().find(|(u, _, _)| *u == best) {
            Some(e) => e.2 += params.lambda,
            None => {
                let raw = human_after[j].moves.iter().find(|(u, _)| *u == best).map(|(_, p)| *p).unwrap_or(0.0);
                set.push((best.clone(), raw, params.lambda));
            }
        }
        for (u, p, w) in set {
            let (h, san) = play(&after[i].0, &u)?;
            if u == best {
                best_reply[i] = Some(san.clone());
            }
            pend.push(Pending { cand: i, is_best: u == best, uci: u, san, p, weight: w, hist: h });
        }
    }

    // 4. Objective value for X after each reply, and V^τ_X from there.
    let pos_of = |q: &Pending| q.hist.last().unwrap().clone();
    let open: Vec<usize> = (0..pend.len()).filter(|&k| terminal_value(&pos_of(&pend[k]), x, params.d).is_none()).collect();
    let open_pos: Vec<Chess> = open.iter().map(|&k| pos_of(&pend[k])).collect();
    let sf_reply = oracle.engine(&open_pos, 1)?;
    let mut objective = vec![0.0; pend.len()];
    for (k, q) in pend.iter().enumerate() {
        if let Some(v) = terminal_value(&pos_of(q), x, params.d) {
            objective[k] = v;
        }
    }
    for (j, &k) in open.iter().enumerate() {
        objective[k] = score_for(x, &open_pos[j], sf_reply[j].lines[0].wdl, params.d);
    }
    let mut value = objective.clone();
    if let (0, LeafEval::Calibrated(m)) = (params.horizon, params.leaf.eval()) {
        // X is to move after the reply: the calibrated human outcome at (X's rating, opponent's).
        for (j, &k) in open.iter().enumerate() {
            value[k] = m.wdl(sf_reply[j].lines[0].pawns(), ratings.of(x), ratings.of(!x), material(&open_pos[j])).score(params.d);
        }
    }
    if params.horizon == 0 && params.leaf == LeafKind::Maia && !open.is_empty() {
        // X is to move after the reply: Maia's outcome estimate at (X's rating, opponent's).
        let hs: Vec<&[Chess]> = open.iter().map(|&k| pend[k].hist.as_slice()).collect();
        let outs = oracle.human(&hs, ratings.of(x), ratings.of(!x))?;
        for (j, &k) in open.iter().enumerate() {
            value[k] = outs[j].wdl.score(params.d);
        }
    }
    if params.horizon > 0 && !open.is_empty() {
        let roots: Vec<(&[Chess], Color)> = open.iter().map(|&k| (pend[k].hist.as_slice(), x)).collect();
        let tp = TreeParams { k: params.horizon, p: params.p, d: params.d, tau: params.tau, leaf: params.leaf.eval() };
        let vals = tree::evaluate_many(oracle, &roots, tp, ratings)?;
        for (j, &k) in open.iter().enumerate() {
            value[k] = vals[j].value;
        }
    }

    // 5. Q(m), objective values, miss chances.
    let mut out: Vec<Candidate> = Vec::new();
    for (i, (u, rank)) in cands.iter().enumerate() {
        let (h, san) = (&after[i].0, after[i].1.clone());
        let pos = h.last().unwrap();
        if let Some(v) = terminal_value(pos, x, params.d) {
            out.push(Candidate { san, uci: u.clone(), p: maia_p(u), engine_rank: *rank, objective: v, practical: v, best_reply: None, miss_chance: 0.0, replies: vec![] });
            continue;
        }
        let j = live.iter().position(|&l| l == i).unwrap();
        let obj = score_for(x, pos, sf_after[j].lines[0].wdl, params.d);
        let mine: Vec<usize> = (0..pend.len()).filter(|&k| pend[k].cand == i).collect();
        let best_obj = mine.iter().find(|&&k| pend[k].is_best).map(|&k| objective[k]).unwrap_or(obj);
        let replies: Vec<Reply> = mine
            .iter()
            .map(|&k| Reply {
                san: pend[k].san.clone(),
                uci: pend[k].uci.clone(),
                p: pend[k].p,
                weight: pend[k].weight,
                objective: objective[k],
                value: value[k],
                is_best: pend[k].is_best,
                miss: !pend[k].is_best && objective[k] > best_obj + params.delta,
            })
            .collect();
        let q = replies.iter().map(|r| r.weight * r.value).sum();
        let miss = replies.iter().filter(|r| r.miss).map(|r| r.p).sum();
        out.push(Candidate { san, uci: u.clone(), p: maia_p(u), engine_rank: *rank, objective: obj, practical: q, best_reply: best_reply[i].clone(), miss_chance: miss, replies });
    }
    out.sort_by(|a, b| b.practical.partial_cmp(&a.practical).unwrap());
    let objective_best = (0..out.len())
        .max_by(|&a, &b| {
            out[a]
                .objective
                .partial_cmp(&out[b].objective)
                .unwrap()
                // ties: Stockfish's own ranking wins
                .then_with(|| out[b].engine_rank.unwrap_or(99).cmp(&out[a].engine_rank.unwrap_or(99)))
        })
        .unwrap();
    let edge = out[0].practical - out[objective_best].practical;
    Ok(Practical {
        fen: full_fen(root),
        side_to_move: turn_name(root).to_string(),
        candidates: out,
        edge,
        clear: objective_best != 0 && edge >= params.margin,
        objective_best,
        practical_best: 0,
        params,
        white_elo: ratings.white,
        black_elo: ratings.black,
        leaf_model: match params.leaf.eval() {
            LeafEval::Calibrated(m) => Some(m.name.to_string()),
            _ => None,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::tests::{hist, Fake};

    const R: Ratings = Ratings { white: 1500.0, black: 1500.0 };

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "{a} != {b}");
    }

    /// Worked by hand (horizon 0, λ = 0.3, d = 0.5). White to move; Black scores are given at
    /// Black-to-move positions, White scores at White-to-move ones.
    ///   1.e4:  objective 0.55; Black always plays the best reply e5 -> Q = 0.55
    ///   1.g4:  objective 0.40 (best reply ...d5); Maia: ...e5 50% (White 0.90), ...d5 50%
    ///          weights: e5 0.7·0.5 = 0.35, d5 0.35 + 0.3 = 0.65
    ///          Q = 0.35·0.90 + 0.65·0.40 = 0.575, miss chance 0.5
    fn fake() -> Fake {
        Fake::default()
            .e("", "e2e4", 0.55)
            .h("", &[("e2e4", 0.5), ("g2g4", 0.5)])
            .e("e2e4", "e7e5", 0.45)
            .e("g2g4", "d7d5", 0.60)
            .h("e2e4", &[("e7e5", 1.0)])
            .h("g2g4", &[("e7e5", 0.5), ("d7d5", 0.5)])
            .e("e2e4 e7e5", "g1f3", 0.55)
            .e("g2g4 d7d5", "g4g5", 0.40)
            .e("g2g4 e7e5", "d2d4", 0.90)
    }

    fn params(lambda: f64) -> PracticalParams {
        // The hand-worked values assume Stockfish leaves.
        PracticalParams { lambda, horizon: 0, leaf: LeafKind::Engine, ..PracticalParams::default() }
    }

    #[test]
    fn hand_worked_q() {
        let r = practical(&fake(), &hist(""), R, params(0.3)).unwrap();
        let g4 = &r.candidates[r.practical_best];
        assert_eq!(g4.san, "g4");
        close(g4.practical, 0.575);
        close(g4.objective, 0.40);
        close(g4.miss_chance, 0.5);
        assert_eq!(g4.best_reply.as_deref(), Some("d5"));
        let e4 = &r.candidates[r.objective_best];
        assert_eq!(e4.san, "e4");
        close(e4.practical, 0.55);
        assert_eq!(e4.engine_rank, Some(1));
        // g4 gains 0.025 over e4: clear at the default margin 0.02, not at 0.03.
        close(r.edge, 0.025);
        assert!(r.clear);
        let strict = practical(&fake(), &hist(""), R, PracticalParams { margin: 0.03, ..params(0.3) }).unwrap();
        assert!(!strict.clear);
    }

    /// λ = 1 is ordinary engine play: Q equals the objective value.
    #[test]
    fn lambda_one_is_engine_play() {
        let r = practical(&fake(), &hist(""), R, params(1.0)).unwrap();
        for c in &r.candidates {
            close(c.practical, c.objective);
        }
        assert_eq!(r.candidates[r.practical_best].san, "e4");
        // Same move: no edge, nothing to show.
        close(r.edge, 0.0);
        assert!(!r.clear);
    }

    /// d = 0 (must-win): draws count for nothing, so every value drops by d·D.
    #[test]
    fn draw_value_changes_scores() {
        let mut p = params(0.3);
        p.d = 0.0;
        let r = practical(&fake(), &hist(""), R, p).unwrap();
        let e4 = r.candidates.iter().find(|c| c.san == "e4").unwrap();
        // White 0.55 at d = 0.5 is W = 0.10, D = 0.90 (see wdl_for), so at d = 0 it is 0.10.
        close(e4.objective, 0.10);
    }

    /// Horizon 1: after the reply, White decides once more with sharpened Maia.
    #[test]
    fn horizon_one_uses_the_tree() {
        let f = fake()
            .h("e2e4 e7e5", &[("g1f3", 1.0)])
            .e("e2e4 e7e5 g1f3", "b8c6", 0.45)
            .h("g2g4 d7d5", &[("g4g5", 1.0)])
            .e("g2g4 d7d5 g4g5", "e7e5", 0.60)
            .h("g2g4 e7e5", &[("d2d4", 1.0)])
            .e("g2g4 e7e5 d2d4", "e5d4", 0.10);
        let p = PracticalParams { horizon: 1, ..params(0.3) };
        let r = practical(&f, &hist(""), R, p).unwrap();
        let g4 = r.candidates.iter().find(|c| c.san == "g4").unwrap();
        close(g4.practical, 0.575);
    }

    /// A mating candidate is worth 1, with no replies.
    #[test]
    fn mate_in_one_candidate() {
        let f = Fake::default()
            .e("f2f3 e7e5 g2g4", "d8h4", 0.99)
            .h("f2f3 e7e5 g2g4", &[("d8h4", 0.4), ("b8c6", 0.6)])
            .e("f2f3 e7e5 g2g4 b8c6", "f1g2", 0.5)
            .h("f2f3 e7e5 g2g4 b8c6", &[("f1g2", 1.0)])
            .e("f2f3 e7e5 g2g4 b8c6 f1g2", "g8f6", 0.5);
        let r = practical(&f, &hist("f2f3 e7e5 g2g4"), R, params(0.3)).unwrap();
        let mate = &r.candidates[r.practical_best];
        assert_eq!(mate.san, "Qh4#");
        close(mate.practical, 1.0);
        assert!(mate.replies.is_empty());
    }
}
