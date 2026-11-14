//! The evaluator interface. Everything the tree and the metrics need comes through
//! [`Oracle`]; real engines, the cache and test fakes all sit behind it.

use serde::{Deserialize, Serialize};
use shakmaty::{fen::Fen, Chess, EnPassantMode, Position};

/// Win/draw/loss probabilities from the side to move's point of view. Sum to 1.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Wdl {
    pub win: f64,
    pub draw: f64,
    pub loss: f64,
}

impl Wdl {
    pub fn new(win: f64, draw: f64, loss: f64) -> Wdl {
        let s = win + draw + loss;
        Wdl { win: win / s, draw: draw / s, loss: loss / s }
    }

    /// Expected score for the side to move, with a draw worth `d`.
    pub fn score(&self, d: f64) -> f64 {
        self.win + d * self.draw
    }

    /// The same probabilities from the other side's point of view.
    pub fn flip(&self) -> Wdl {
        Wdl { win: self.loss, draw: self.draw, loss: self.win }
    }
}

/// One line of a Stockfish search.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EngineLine {
    pub uci: String,
    /// From the side to move at the searched position.
    pub wdl: Wdl,
    pub depth: u32,
    /// Stockfish's evaluation in centipawns, side to move (absent in older cache entries).
    #[serde(default)]
    pub cp: Option<i32>,
    /// Mate in N (positive: side to move mates).
    #[serde(default)]
    pub mate: Option<i32>,
}

impl EngineLine {
    /// Evaluation in pawns for the side to move, capped at ±10; a mate counts as ±12.
    /// Falls back to the W/D/L's logit if the line has no score (old cache entries).
    pub fn pawns(&self) -> f64 {
        if let Some(m) = self.mate {
            return if m > 0 { 12.0 } else { -12.0 };
        }
        if let Some(cp) = self.cp {
            return (cp as f64 / 100.0).clamp(-10.0, 10.0);
        }
        let ve = (self.wdl.win + 0.5 * self.wdl.draw).clamp(1e-3, 1.0 - 1e-3);
        (ve / (1.0 - ve)).ln().clamp(-10.0, 10.0)
    }
}

/// A Stockfish search result; `lines[0]` is the best move. Never empty for a position with
/// legal moves.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EngineResult {
    pub lines: Vec<EngineLine>,
}

/// Maia's output at one position.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HumanResult {
    /// Legal moves with probabilities, sorted by probability, highest first.
    pub moves: Vec<(String, f64)>,
    /// Maia's value head, from the side to move.
    pub wdl: Wdl,
}

pub trait Oracle {
    /// Stockfish searches of `positions`, with `multipv` lines each. Positions must not be
    /// game-over.
    fn engine(&self, positions: &[Chess], multipv: usize) -> anyhow::Result<Vec<EngineResult>>;

    /// Maia at each history's last position. A history is oldest..current; only the last 8
    /// positions matter. `elo_self` is the rating of the side to move.
    fn human(&self, histories: &[&[Chess]], elo_self: f32, elo_oppo: f32) -> anyhow::Result<Vec<HumanResult>>;
}

/// FEN without the move counters: the position identity used for Stockfish caching.
pub fn fen4(pos: &Chess) -> String {
    let fen = Fen::from_position(pos, EnPassantMode::Legal).to_string();
    fen.split(' ').take(4).collect::<Vec<_>>().join(" ")
}

pub fn full_fen(pos: &Chess) -> String {
    Fen::from_position(pos, EnPassantMode::Legal).to_string()
}

pub fn turn_name(pos: &Chess) -> &'static str {
    match pos.turn() {
        shakmaty::Color::White => "white",
        shakmaty::Color::Black => "black",
    }
}
