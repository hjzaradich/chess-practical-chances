//! Spike 1b: run Maia3-5M (our history-input ONNX export) from Rust with `ort`, doing the
//! input encoding ourselves with shakmaty, and compare against the official Python package's
//! outputs in ../s1_reference.json.
//!
//! cargo run --release -- ../_dl/models/maia3-5m.history.fp32.onnx ../s1_reference.json

use std::collections::HashMap;
use std::time::Instant;

use ort::session::Session;
use ort::value::Tensor;
use serde::Deserialize;
use shakmaty::fen::Fen;
use shakmaty::uci::UciMove;
use shakmaty::{CastlingMode, Chess, Color, Position, Role, Square};

const HISTORY: usize = 8;
const VOCAB: usize = 4352;

#[derive(Deserialize)]
struct Case {
    name: String,
    start_fen: Option<String>,
    moves: Vec<String>,
    elo: f32,
    probs: HashMap<String, f64>,
    wdl: [f64; 3],
}

fn move_vocab() -> HashMap<String, usize> {
    let files = b"abcdefgh";
    let mut v = Vec::with_capacity(VOCAB);
    for from in 0..64u32 {
        for to in 0..64u32 {
            let f = Square::new(from);
            let t = Square::new(to);
            v.push(format!("{f}{t}"));
        }
    }
    for ff in files {
        for tf in files {
            for p in ["q", "r", "b", "n"] {
                v.push(format!("{}7{}8{}", *ff as char, *tf as char, p));
            }
        }
    }
    assert_eq!(v.len(), VOCAB);
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

/// (64, 12) one-hot, mirrored (flip ranks + swap colours) when black is to move.
fn tokenize(pos: &Chess, out: &mut [f32], stride: usize, offset: usize) {
    let black = pos.turn() == Color::Black;
    for (sq, piece) in pos.board().clone() {
        let (sq, color) = if black { (sq.flip_vertical(), !piece.color) } else { (sq, piece.color) };
        let ch = role_index(piece.role) + if color == Color::Black { 6 } else { 0 };
        out[usize::from(sq) * stride + offset + ch] = 1.0;
    }
}

/// history: oldest..current. Writes (64, 96) into out; front-pads with the earliest position.
fn history_tokens(history: &[Chess], out: &mut [f32]) {
    let h = &history[history.len().saturating_sub(HISTORY)..];
    let pad = HISTORY - h.len();
    for slot in 0..HISTORY {
        let p = if slot < pad { &h[0] } else { &h[slot - pad] };
        tokenize(p, out, 12 * HISTORY, 12 * slot);
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

fn build_history(case: &Case) -> Vec<Chess> {
    let start: Chess = match &case.start_fen {
        Some(f) => f.parse::<Fen>().unwrap().into_position(CastlingMode::Standard).unwrap(),
        None => Chess::default(),
    };
    let mut hist = vec![start.clone()];
    let mut pos = start;
    for u in &case.moves {
        let m = u.parse::<UciMove>().unwrap().to_move(&pos).unwrap();
        pos.play_unchecked(m);
        hist.push(pos.clone());
    }
    hist
}

fn run(session: &mut Session, hist: &[f32], elo_s: &[f32], elo_o: &[f32], n: usize) -> (Vec<f32>, Vec<f32>) {
    let h = Tensor::from_array(([n, 64, 12 * HISTORY], hist.to_vec())).unwrap();
    let es = Tensor::from_array(([n], elo_s.to_vec())).unwrap();
    let eo = Tensor::from_array(([n], elo_o.to_vec())).unwrap();
    let outputs = session
        .run(ort::inputs!["history" => h, "elo_self" => es, "elo_oppo" => eo])
        .unwrap();
    let (_, lm) = outputs["logits_move"].try_extract_tensor::<f32>().unwrap();
    let (_, lv) = outputs["logits_value"].try_extract_tensor::<f32>().unwrap();
    (lm.to_vec(), lv.to_vec())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let model = &args[1];
    let cases: Vec<Case> = serde_json::from_str(&std::fs::read_to_string(&args[2]).unwrap()).unwrap();
    let vocab = move_vocab();

    let env = ort::init().build().unwrap();
    let mut session = Session::builder(&env)
        .unwrap()
        .with_intra_threads(1)
        .unwrap()
        .commit_from_file(model)
        .unwrap();

    let (mut max_dp, mut max_dv) = (0.0f64, 0.0f64);
    let mut top1_agree = 0;
    let mut missing = 0;
    for case in &cases {
        let hist = build_history(case);
        let pos = hist.last().unwrap();
        let mut toks = vec![0f32; 64 * 12 * HISTORY];
        history_tokens(&hist, &mut toks);
        let (lm, lv) = run(&mut session, &toks, &[case.elo], &[case.elo], 1);

        let legal: Vec<String> = pos
            .legal_moves()
            .iter()
            .map(|m| m.to_uci(CastlingMode::Standard).to_string())
            .collect();
        let logits: Vec<f64> = legal
            .iter()
            .map(|u| {
                let key = if pos.turn() == Color::Black { mirror_uci(u) } else { u.clone() };
                lm[vocab[&key]] as f64
            })
            .collect();
        let p = softmax(&logits);
        let v = softmax(&lv.iter().map(|&x| x as f64).collect::<Vec<_>>()); // loss, draw, win
        let wdl = [v[2], v[1], v[0]];

        if legal.len() != case.probs.len() {
            println!("{}: legal-move count differs: rust {} python {}", case.name, legal.len(), case.probs.len());
        }
        let mut best_r = (String::new(), -1.0);
        for (u, pr) in legal.iter().zip(&p) {
            match case.probs.get(u) {
                Some(py) => max_dp = max_dp.max((pr - py).abs()),
                None => missing += 1,
            }
            if *pr > best_r.1 {
                best_r = (u.clone(), *pr);
            }
        }
        let best_py = case.probs.iter().max_by(|a, b| a.1.partial_cmp(b.1).unwrap()).unwrap();
        if &best_r.0 == best_py.0 {
            top1_agree += 1;
        }
        for i in 0..3 {
            max_dv = max_dv.max((wdl[i] - case.wdl[i]).abs());
        }
    }
    println!("cases: {}", cases.len());
    println!("max |P_rust - P_python| over all legal moves: {max_dp:.2e}");
    println!("max |WDL_rust - WDL_python|: {max_dv:.2e}");
    println!("top-1 move agreement: {top1_agree}/{}", cases.len());
    println!("moves Rust generated that Python lacked: {missing}");

    // ---- throughput: single positions and batches
    let case = &cases[2];
    let hist = build_history(case);
    let mut toks = vec![0f32; 64 * 12 * HISTORY];
    history_tokens(&hist, &mut toks);
    for &batch in &[1usize, 8, 32, 64] {
        let mut big = Vec::with_capacity(batch * toks.len());
        for _ in 0..batch {
            big.extend_from_slice(&toks);
        }
        let e = vec![1500f32; batch];
        run(&mut session, &big, &e, &e, batch); // warm-up
        let reps = (256 / batch).max(4);
        let t0 = Instant::now();
        for _ in 0..reps {
            run(&mut session, &big, &e, &e, batch);
        }
        let per_pos = t0.elapsed().as_secs_f64() / (reps * batch) as f64;
        println!("batch {batch:3}: {:.3} ms per position (1 thread)", per_pos * 1e3);
    }
}
