//! PGN loading and the per-game driver.

use std::collections::HashMap;
use std::ops::ControlFlow;

use anyhow::{anyhow, Context};
use pgn_reader::{RawTag, Reader, SanPlus, Visitor};
use shakmaty::{fen::Fen, san::San, uci::UciMove, CastlingMode, Chess, Position};

use crate::analysis::{analyze_ply, PlyResult, Settings};
use crate::oracle::Oracle;
use crate::tree::HISTORY;

pub struct Game {
    pub tags: HashMap<String, String>,
    /// positions[0] is the start; positions[i] is after i half-moves.
    pub positions: Vec<Chess>,
    /// (SAN, UCI) of the move that led to positions[i + 1].
    pub moves: Vec<(String, String)>,
}

impl Game {
    pub fn tag(&self, name: &str) -> Option<&str> {
        self.tags.get(name).map(|s| s.as_str())
    }

    /// A rating from a header like WhiteElo, if it's a number.
    pub fn elo(&self, name: &str) -> Option<f32> {
        self.tag(name)?.trim().parse::<f32>().ok().filter(|e| *e > 0.0)
    }

    pub fn from_start(start: Chess, sans: &[San]) -> anyhow::Result<Game> {
        let mut positions = vec![start];
        let mut moves = Vec::new();
        for (i, san) in sans.iter().enumerate() {
            let pos = positions.last().unwrap();
            let m = san.to_move(pos).map_err(|e| anyhow!("illegal move {} at ply {}: {e}", san, i + 1))?;
            let uci = UciMove::from_move(m.clone(), CastlingMode::Standard).to_string();
            let san_plus = shakmaty::san::SanPlus::from_move(pos.clone(), m.clone());
            let mut next = pos.clone();
            next.play_unchecked(m);
            moves.push((san_plus.to_string(), uci));
            positions.push(next);
        }
        Ok(Game { tags: HashMap::new(), positions, moves })
    }
}

#[derive(Default)]
struct Collector;

struct Movetext {
    tags: HashMap<String, String>,
    sans: Vec<San>,
}

impl Visitor for Collector {
    type Tags = HashMap<String, String>;
    type Movetext = Movetext;
    type Output = Movetext;

    fn begin_tags(&mut self) -> ControlFlow<Self::Output, Self::Tags> {
        ControlFlow::Continue(HashMap::new())
    }

    fn tag(&mut self, tags: &mut Self::Tags, name: &[u8], value: RawTag<'_>) -> ControlFlow<Self::Output> {
        tags.insert(String::from_utf8_lossy(name).into_owned(), value.decode_utf8_lossy().into_owned());
        ControlFlow::Continue(())
    }

    fn begin_movetext(&mut self, tags: Self::Tags) -> ControlFlow<Self::Output, Self::Movetext> {
        ControlFlow::Continue(Movetext { tags, sans: Vec::new() })
    }

    fn san(&mut self, mt: &mut Self::Movetext, san_plus: SanPlus) -> ControlFlow<Self::Output> {
        mt.sans.push(san_plus.san);
        ControlFlow::Continue(())
    }

    // Variations are skipped by default: only the main line is analysed.

    fn end_game(&mut self, mt: Self::Movetext) -> Self::Output {
        mt
    }
}

fn build(mt: Movetext) -> anyhow::Result<Game> {
    let start = match mt.tags.get("FEN") {
        Some(f) => f
            .parse::<Fen>()
            .context("bad FEN header")?
            .into_position(CastlingMode::Standard)
            .context("illegal FEN header")?,
        None => Chess::default(),
    };
    let mut g = Game::from_start(start, &mt.sans)?;
    g.tags = mt.tags;
    Ok(g)
}

/// Positions from a start FEN through SAN moves ("1. e4 e5 2. Nf3" works).
pub fn history_from(fen: Option<&str>, moves: &str) -> anyhow::Result<Vec<Chess>> {
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
            .parse::<San>()
            .map_err(|e| anyhow::anyhow!("{t}: {e}"))?
            .to_move(pos)
            .map_err(|e| anyhow::anyhow!("{t}: {e}"))?;
        let mut next = pos.clone();
        next.play_unchecked(m);
        hist.push(next);
    }
    Ok(hist)
}

/// The `index`-th game (0-based) in a PGN text.
pub fn parse_pgn(text: &str, index: usize) -> anyhow::Result<Game> {
    let mut reader = Reader::new(text.as_bytes());
    let mut v = Collector;
    let mut n = 0;
    while let Some(mt) = reader.read_game(&mut v)? {
        if n == index {
            return build(mt);
        }
        n += 1;
    }
    Err(anyhow!("the PGN has {n} game(s); game {} not found", index + 1))
}

/// Every game in a PGN text; a game that fails to parse is returned as an error in place.
pub fn parse_all(text: &str) -> anyhow::Result<Vec<anyhow::Result<Game>>> {
    let mut reader = Reader::new(text.as_bytes());
    let mut v = Collector;
    let mut out = Vec::new();
    while let Some(mt) = reader.read_game(&mut v)? {
        out.push(build(mt));
    }
    Ok(out)
}

/// Analyse every ply of `game` in order, handing each result to `emit` as soon as it's done.
/// `emit` returns false to cancel.
pub fn analyze_game(
    oracle: &dyn Oracle,
    game: &Game,
    settings: &Settings,
    mut emit: impl FnMut(&PlyResult) -> anyhow::Result<bool>,
) -> anyhow::Result<()> {
    for ply in 0..game.positions.len() {
        let start = (ply + 1).saturating_sub(HISTORY);
        let hist = &game.positions[start..=ply];
        let last = if ply == 0 { None } else { Some(game.moves[ply - 1].clone()) };
        let r = analyze_ply(oracle, hist, ply as u32, last, settings)
            .with_context(|| format!("analysing ply {ply}"))?;
        if !emit(&r)? {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mainline_tags_and_skips_variations() {
        let pgn = r#"[Event "Test"]
[White "A"]
[Black "B"]
[WhiteElo "1612"]
[BlackElo "?"]

1. e4 e5 (1... c5 2. Nf3) 2. Nf3 {a comment} Nc6 3. Bb5 a6 4. O-O 1-0
"#;
        let g = parse_pgn(pgn, 0).unwrap();
        assert_eq!(g.elo("WhiteElo"), Some(1612.0));
        assert_eq!(g.elo("BlackElo"), None);
        assert_eq!(g.positions.len(), 8);
        assert_eq!(g.moves[6], ("O-O".to_string(), "e1g1".to_string()));
        assert!(parse_pgn(pgn, 1).is_err());
    }

    #[test]
    fn fen_header_start() {
        let pgn = "[FEN \"8/8/8/8/8/8/k7/2K4R w - - 0 1\"]\n[SetUp \"1\"]\n\n1. Rh2+ Ka1 *\n";
        let g = parse_pgn(pgn, 0).unwrap();
        assert_eq!(g.moves[0].0, "Rh2+");
        assert_eq!(g.positions.len(), 3);
    }
}
