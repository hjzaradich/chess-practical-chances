//! How tree leaves are scored (DECISIONS 4.5).
//!
//! T uses Stockfish's W/D/L as is (SPEC.md's v). Phase 4's practical value scores leaves with
//! a rating-calibrated model: Stockfish's evaluation mapped to the outcome of a *human* game
//! between the two ratings, fitted on Lichess games (`validation/fit_leaf_model.py`).

use serde::{Deserialize, Serialize};
use shakmaty::{Chess, Position, Role};

use crate::oracle::Wdl;

/// Material on the board (P=1, N=B=3, R=5, Q=9), both sides together.
pub fn material(pos: &Chess) -> u32 {
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

/// Ordered logit, symmetric between the sides (see validation/fit_leaf_model.py):
///   x = Stockfish's evaluation in pawns (±10 cap, mate ±12), saturated as scale·tanh(x/scale)
///   when scale > 0, z = (mean rating − 1500)/400, dr = (rating to move − other)/400,
///   m = (material − 40)/40, eta = (a0 + a1 z + a2 m) x + g dr, h = exp(w0 + w1 z + w2 m),
///   P(win) = σ(eta − h), P(loss) = σ(−eta − h).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LeafModel {
    pub name: &'static str,
    pub a0: f64,
    pub a1: f64,
    pub a2: f64,
    pub g: f64,
    pub w0: f64,
    pub w1: f64,
    pub w2: f64,
    /// Saturation of the evaluation: x̃ = scale·tanh(x/scale); 0 = use x as is.
    pub scale: f64,
}

fn sigmoid(t: f64) -> f64 {
    1.0 / (1.0 + (-t).exp())
}

impl LeafModel {
    /// Human-game W/D/L for the side to move, from Stockfish's evaluation in pawns
    /// ([`crate::oracle::EngineLine::pawns`]) at the position.
    pub fn wdl(&self, pawns: f64, rating_to_move: f32, rating_other: f32, material: u32) -> Wdl {
        let x = if self.scale > 0.0 { self.scale * (pawns / self.scale).tanh() } else { pawns };
        let z = ((rating_to_move + rating_other) as f64 / 2.0 - 1500.0) / 400.0;
        let dr = (rating_to_move - rating_other) as f64 / 400.0;
        let m = (material as f64 - 40.0) / 40.0;
        let eta = (self.a0 + self.a1 * z + self.a2 * m) * x + self.g * dr;
        let h = (self.w0 + self.w1 * z + self.w2 * m).exp();
        let win = sigmoid(eta - h);
        let loss = sigmoid(-eta - h);
        Wdl { win, draw: (1.0 - win - loss).max(0.0), loss }
    }
}

/// Fitted on Lichess 2026-10: 8,000 games, ratings 800-2399, plies 12/24/36/48, 28,583 positions
/// (70% fit; held-out log loss 0.80 vs 7.5 for Stockfish's W/D/L, Brier 0.211 vs 0.268).
/// Values from validation/results/calib/leaf_model.json.
pub const LICHESS_2026_10: LeafModel = LeafModel {
    name: "lichess-2026-10-ordered-logit-v1",
    a0: 0.21966803878176897,
    a1: 0.03166203100338028,
    a2: -0.10179680674359796,
    g: 1.133533956876571,
    w0: -1.6124328490125914,
    w1: 0.08852576500610167,
    w2: -1.008706145234875,
    scale: 0.0,
};

/// Which value a tree leaf gets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LeafEval {
    /// Stockfish's W/D/L (T, and SPEC.md's literal Q).
    Engine,
    /// Maia's value head at the two ratings (experiment, DECISIONS 4.3).
    Maia,
    /// Stockfish's W/D/L through a rating-calibrated model (Phase 4 default, DECISIONS 4.5).
    Calibrated(LeafModel),
}

/// Serializable name of the leaf setting, for results and the CLI.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LeafKind {
    Engine,
    Maia,
    Calibrated,
}

impl LeafKind {
    pub fn eval(self) -> LeafEval {
        match self {
            LeafKind::Engine => LeafEval::Engine,
            LeafKind::Maia => LeafEval::Maia,
            LeafKind::Calibrated => LeafEval::Calibrated(LICHESS_2026_10),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const M: LeafModel = LeafModel { name: "test", a0: 0.5, a1: 0.1, a2: 0.0, g: 0.6, w0: -1.0, w1: 0.2, w2: 0.0, scale: 4.0 };

    #[test]
    fn symmetric_between_sides() {
        let a = M.wdl(1.3, 1500.0, 1300.0, 50);
        let b = M.wdl(-1.3, 1300.0, 1500.0, 50);
        assert!((a.win - b.loss).abs() < 1e-12 && (a.draw - b.draw).abs() < 1e-12);
    }

    #[test]
    fn probabilities_sum_to_one_and_track_the_engine() {
        let even = M.wdl(0.0, 1500.0, 1500.0, 40);
        assert!((even.win - even.loss).abs() < 1e-12);
        let up = M.wdl(2.5, 1500.0, 1500.0, 40);
        assert!(up.win > even.win);
        for w in [even, up] {
            assert!((w.win + w.draw + w.loss - 1.0).abs() < 1e-12);
        }
    }

    /// Matches the fit's own table: +3 pawns at 1500 vs 1500, material 40 -> 0.66.
    #[test]
    fn fitted_model_matches_fit_table() {
        let w = LICHESS_2026_10.wdl(3.0, 1500.0, 1500.0, 40);
        assert!((w.win + 0.5 * w.draw - 0.66).abs() < 0.006, "{:?}", w);
        let low = LICHESS_2026_10.wdl(3.0, 800.0, 800.0, 40);
        let high = LICHESS_2026_10.wdl(3.0, 2300.0, 2300.0, 40);
        assert!(low.win < w.win && w.win < high.win);
    }

    #[test]
    fn material_of_start_position() {
        assert_eq!(material(&Chess::default()), 78);
    }
}
