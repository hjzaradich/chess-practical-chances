//! Maia-3 through ONNX Runtime. Input encoding ported from the official CSSLab/maia3
//! package (AGPL-3.0) and checked against it in Phase 0 (spikes/s1_maia_rust).
//!
//! Expects our export with history as an input (tools/export_maia_onnx.py):
//!   history  f32 [B, 64, 96]   8 slots × 12 piece channels, oldest first
//!   elo_self f32 [B], elo_oppo f32 [B]
//!   -> logits_move f32 [B, 4352], logits_value f32 [B, 3] as [loss, draw, win]

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use anyhow::{anyhow, Context};
use ort::session::Session;
use ort::value::Tensor;
use sha2::{Digest, Sha256};
use shakmaty::{CastlingMode, Chess, Color, Position, Role, Square};

use crate::oracle::{HumanResult, Wdl};
use crate::tree::HISTORY;

const VOCAB: usize = 4352;
const WIDTH: usize = 12 * HISTORY;
const MAX_BATCH: usize = 64;

pub struct Maia {
    session: Mutex<Session>,
    vocab: HashMap<String, usize>,
    /// File name + first 12 hex digits of its SHA-256.
    pub id: String,
}

fn move_vocab() -> HashMap<String, usize> {
    let mut v = Vec::with_capacity(VOCAB);
    for from in 0..64u32 {
        for to in 0..64u32 {
            v.push(format!("{}{}", Square::new(from), Square::new(to)));
        }
    }
    for ff in "abcdefgh".chars() {
        for tf in "abcdefgh".chars() {
            for p in ["q", "r", "b", "n"] {
                v.push(format!("{ff}7{tf}8{p}"));
            }
        }
    }
    v.into_iter().enumerate().map(|(i, m)| (m, i)).collect()
}

fn role_index(r: Role) -> usize {
    match r {
        Role::Pawn => 0,
        Role::Knight => 1,
        Role::Bishop => 2,
        Role::Rook => 3,
        Role::Queen => 4,
        Role::King => 5,
    }
}

/// One slot: 64 × 12 one-hot, mirrored (ranks flipped, colours swapped) when Black is to move.
fn tokenize(pos: &Chess, out: &mut [f32], slot: usize) {
    let black = pos.turn() == Color::Black;
    for (sq, piece) in pos.board().clone() {
        let (sq, color) = if black { (sq.flip_vertical(), !piece.color) } else { (sq, piece.color) };
        let ch = role_index(piece.role) + if color == Color::Black { 6 } else { 0 };
        out[usize::from(sq) * WIDTH + 12 * slot + ch] = 1.0;
    }
}

/// (64, 96) for a history (oldest..current), front-padded with the earliest position.
pub fn encode_history(history: &[Chess], out: &mut [f32]) {
    let h = &history[history.len().saturating_sub(HISTORY)..];
    let pad = HISTORY - h.len();
    for slot in 0..HISTORY {
        tokenize(if slot < pad { &h[0] } else { &h[slot - pad] }, out, slot);
    }
}

fn mirror_uci(u: &str) -> String {
    let b = u.as_bytes();
    let flip = |c: u8| (b'1' + b'8' - c) as char;
    format!("{}{}{}{}{}", b[0] as char, flip(b[1]), b[2] as char, flip(b[3]), &u[4..])
}

fn softmax(x: &[f64]) -> Vec<f64> {
    let m = x.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let e: Vec<f64> = x.iter().map(|v| (v - m).exp()).collect();
    let s: f64 = e.iter().sum();
    e.into_iter().map(|v| v / s).collect()
}

impl Maia {
    pub fn load(path: &Path, threads: usize) -> anyhow::Result<Maia> {
        let bytes = std::fs::read(path).with_context(|| format!("reading Maia model {}", path.display()))?;
        let digest = Sha256::digest(&bytes);
        let hex: String = digest.iter().take(6).map(|b| format!("{b:02x}")).collect();
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let env = ort::init().build().map_err(|e| anyhow!("onnxruntime init: {e}"))?;
        let err = |e: &dyn std::fmt::Display| anyhow!("loading Maia model: {e}");
        let mut builder = Session::builder(&env)
            .map_err(|e| err(&e))?
            .with_intra_threads(threads)
            .map_err(|e| err(&e))?;
        let session = builder.commit_from_memory(&bytes).map_err(|e| err(&e))?;
        let names: Vec<String> = session.inputs().iter().map(|i| i.name().to_string()).collect();
        if !names.iter().any(|n| n == "history") {
            return Err(anyhow!(
                "{} has inputs {names:?}; expected a history-input export (tools/export_maia_onnx.py)",
                path.display()
            ));
        }
        Ok(Maia { session: Mutex::new(session), vocab: move_vocab(), id: format!("{name}@{hex}") })
    }

    /// Maia at each history's last position, all with the same rating pair.
    pub fn evaluate(&self, histories: &[&[Chess]], elo_self: f32, elo_oppo: f32) -> anyhow::Result<Vec<HumanResult>> {
        let mut out = Vec::with_capacity(histories.len());
        for chunk in histories.chunks(MAX_BATCH) {
            let n = chunk.len();
            let mut toks = vec![0f32; n * 64 * WIDTH];
            for (i, h) in chunk.iter().enumerate() {
                encode_history(h, &mut toks[i * 64 * WIDTH..(i + 1) * 64 * WIDTH]);
            }
            let (lm, lv) = {
                let mut s = self.session.lock().unwrap();
                let outputs = s
                    .run(ort::inputs![
                        "history" => Tensor::from_array(([n, 64, WIDTH], toks))?,
                        "elo_self" => Tensor::from_array(([n], vec![elo_self; n]))?,
                        "elo_oppo" => Tensor::from_array(([n], vec![elo_oppo; n]))?
                    ])
                    .map_err(|e| anyhow!("Maia inference: {e}"))?;
                let (_, lm) = outputs["logits_move"].try_extract_tensor::<f32>()?;
                let (_, lv) = outputs["logits_value"].try_extract_tensor::<f32>()?;
                (lm.to_vec(), lv.to_vec())
            };
            for (i, h) in chunk.iter().enumerate() {
                let pos = h.last().unwrap();
                let legal: Vec<String> =
                    pos.legal_moves().iter().map(|m| m.to_uci(CastlingMode::Standard).to_string()).collect();
                let logits: Vec<f64> = legal
                    .iter()
                    .map(|u| {
                        let key = if pos.turn() == Color::Black { mirror_uci(u) } else { u.clone() };
                        lm[i * VOCAB + self.vocab[&key]] as f64
                    })
                    .collect();
                let probs = if legal.is_empty() { vec![] } else { softmax(&logits) };
                let mut moves: Vec<(String, f64)> = legal.into_iter().zip(probs).collect();
                moves.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
                let v = softmax(&lv[i * 3..i * 3 + 3].iter().map(|&x| x as f64).collect::<Vec<_>>());
                out.push(HumanResult { moves, wdl: Wdl::new(v[2], v[1], v[0]) });
            }
        }
        Ok(out)
    }
}
