//! Chess trickiness analysis core. See SPEC.md.
//!
//! - [`oracle`]: the evaluator interface (Stockfish + Maia behind one trait).
//! - [`tree`] and [`analysis`]: the metrics. Pure; tested with fakes.
//! - [`stockfish`], [`maia`], [`cache`], [`engines`]: the real evaluator behind the interface.
//! - [`game`]: PGN loading and the per-game driver that streams one result per ply.

pub mod analysis;
pub mod cache;
pub mod engines;
pub mod game;
pub mod leaf;
pub mod maia;
pub mod oracle;
pub mod practical;
pub mod stockfish;
pub mod tree;
