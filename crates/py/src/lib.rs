//! Python bindings for trickiness-core, imported as `practical_chances._core`.
//!
//! This layer is deliberately thin. It starts the engines, runs the analysis with the GIL
//! released, and hands results back as JSON in the same serde schema the desktop app uses.
//! The typed public API (dataclasses, pandas, model download) lives in the Python package in
//! `python/src/practical_chances`, and `_core.pyi` there describes this module.

use std::path::PathBuf;
use std::sync::RwLock;

use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyValueError};
use pyo3::prelude::*;
use trickiness_core::analysis::{analyze_ply, Settings};
use trickiness_core::engines::EngineOracle;
use trickiness_core::game::{analyze_game, history_from, parse_all, Game};
use trickiness_core::leaf::LeafKind;
use trickiness_core::oracle::full_fen;
use trickiness_core::practical::{practical, PracticalParams};
use trickiness_core::tree::Ratings;

create_exception!(_core, EngineError, PyException, "Stockfish, Maia or the analysis itself failed.");

fn engine_err(e: anyhow::Error) -> PyErr {
    EngineError::new_err(format!("{e:#}"))
}

fn value_err(e: anyhow::Error) -> PyErr {
    PyValueError::new_err(format!("{e:#}"))
}

/// Running Stockfish processes plus the Maia model and the evaluation cache.
///
/// Starting the engines is the slow part, so one `Engine` serves many analyses. Methods
/// release the GIL while they work, so other Python threads keep running, and several
/// threads may analyse with the same engine at once.
#[pyclass(module = "practical_chances._core", frozen)]
struct Engine {
    /// `None` after `close()`. Analyses hold the read lock; `close()` takes the write lock,
    /// so it waits for running analyses instead of pulling the engines out from under them.
    oracle: RwLock<Option<EngineOracle>>,
    stockfish_name: String,
    maia_id: String,
    nodes: u64,
}

/// The settings a ply analysis records alongside its numbers (traceability, SPEC.md).
struct PlyOpts {
    k: u32,
    p: f64,
    d: f64,
    epsilon: f64,
    delta: f64,
    multipv: usize,
}

impl Engine {
    fn with_oracle<R>(&self, f: impl FnOnce(&EngineOracle) -> PyResult<R>) -> PyResult<R> {
        let guard = self.oracle.read().map_err(|_| EngineError::new_err("engine lock poisoned"))?;
        let oracle = guard.as_ref().ok_or_else(|| EngineError::new_err("this Engine has been closed"))?;
        f(oracle)
    }

    fn settings(&self, o: &PlyOpts, white_elo: f32, black_elo: f32) -> PyResult<Settings> {
        if o.k == 0 {
            return Err(PyValueError::new_err("k must be at least 1"));
        }
        if !(o.p > 0.0 && o.p <= 1.0) {
            return Err(PyValueError::new_err("p must be in (0, 1]"));
        }
        Ok(Settings {
            k: o.k,
            p: o.p,
            nodes: self.nodes,
            d: o.d,
            epsilon: o.epsilon,
            delta: o.delta,
            multipv: o.multipv,
            white_elo,
            black_elo,
            stockfish: self.stockfish_name.clone(),
            maia: self.maia_id.clone(),
        })
    }
}

#[pymethods]
impl Engine {
    #[new]
    #[pyo3(signature = (stockfish, maia_model, *, nodes = 50_000, workers = None, hash_mb = 16, syzygy = None, cache = None))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        py: Python<'_>,
        stockfish: PathBuf,
        maia_model: PathBuf,
        nodes: u64,
        workers: Option<usize>,
        hash_mb: u32,
        syzygy: Option<PathBuf>,
        cache: Option<PathBuf>,
    ) -> PyResult<Self> {
        let workers = workers
            .unwrap_or_else(|| std::thread::available_parallelism().map(|n| (n.get() / 2).max(1)).unwrap_or(2));
        if workers == 0 {
            return Err(PyValueError::new_err("workers must be at least 1"));
        }
        let oracle = py
            .detach(|| {
                EngineOracle::start(&stockfish, workers, nodes, hash_mb, syzygy.as_deref(), &maia_model, cache.as_deref())
            })
            .map_err(engine_err)?;
        Ok(Engine {
            stockfish_name: oracle.stockfish.name.clone(),
            maia_id: oracle.maia.id.clone(),
            nodes,
            oracle: RwLock::new(Some(oracle)),
        })
    }

    /// Stockfish's name as it reports it, plus tablebases if in use.
    #[getter]
    fn stockfish_name(&self) -> &str {
        &self.stockfish_name
    }

    /// Model file name and the start of its SHA-256.
    #[getter]
    fn maia_id(&self) -> &str {
        &self.maia_id
    }

    #[getter]
    fn nodes(&self) -> u64 {
        self.nodes
    }

    /// (hits, misses) of the evaluation cache since the engine started.
    fn cache_stats(&self) -> PyResult<(u64, u64)> {
        self.with_oracle(|o| {
            let hits = *o.cache.hits.lock().unwrap();
            let misses = *o.cache.misses.lock().unwrap();
            Ok((hits, misses))
        })
    }

    /// Analyse one position: a start FEN (default: the initial position) and SAN moves from it.
    /// Returns a PlyResult as JSON.
    #[pyo3(signature = (*, fen = None, moves = "", white_elo, black_elo, k = 2, p = 0.9, d = 0.5, epsilon = 0.01, delta = 0.05, multipv = 5))]
    #[allow(clippy::too_many_arguments)]
    fn analyze_position(
        &self,
        py: Python<'_>,
        fen: Option<&str>,
        moves: &str,
        white_elo: f32,
        black_elo: f32,
        k: u32,
        p: f64,
        d: f64,
        epsilon: f64,
        delta: f64,
        multipv: usize,
    ) -> PyResult<String> {
        let settings = self.settings(&PlyOpts { k, p, d, epsilon, delta, multipv }, white_elo, black_elo)?;
        let hist = history_from(fen, moves).map_err(value_err)?;
        self.with_oracle(|o| {
            py.detach(|| {
                let ply = (hist.len() - 1) as u32;
                let r = analyze_ply(o, &hist, ply, None, &settings)?;
                Ok(serde_json::to_string(&r)?)
            })
            .map_err(engine_err)
        })
    }

    /// Analyse every ply of game `game` (0-based) in a PGN text. Returns one PlyResult JSON per
    /// ply. `on_ply` is called with each one as it finishes; if it raises, or Ctrl+C is pressed,
    /// the analysis stops and the exception propagates.
    #[pyo3(signature = (pgn, *, game = 0, white_elo = None, black_elo = None, k = 2, p = 0.9, d = 0.5, epsilon = 0.01, delta = 0.05, multipv = 5, on_ply = None))]
    #[allow(clippy::too_many_arguments)]
    fn analyze_game(
        &self,
        py: Python<'_>,
        pgn: &str,
        game: usize,
        white_elo: Option<f32>,
        black_elo: Option<f32>,
        k: u32,
        p: f64,
        d: f64,
        epsilon: f64,
        delta: f64,
        multipv: usize,
        on_ply: Option<Py<PyAny>>,
    ) -> PyResult<Vec<String>> {
        let g = trickiness_core::game::parse_pgn(pgn, game).map_err(value_err)?;
        let we = white_elo.or(g.elo("WhiteElo")).unwrap_or(1500.0);
        let be = black_elo.or(g.elo("BlackElo")).unwrap_or(1500.0);
        let settings = self.settings(&PlyOpts { k, p, d, epsilon, delta, multipv }, we, be)?;
        self.with_oracle(|o| {
            let mut out = Vec::with_capacity(g.positions.len());
            // A Python exception raised in the callback (or a pending Ctrl+C) is kept here and
            // re-raised as itself, rather than flattened into an EngineError.
            let mut py_err: Option<PyErr> = None;
            let res = py.detach(|| {
                analyze_game(o, &g, &settings, |r| {
                    let json = serde_json::to_string(r)?;
                    let stop = Python::attach(|py| -> PyResult<()> {
                        py.check_signals()?;
                        if let Some(cb) = &on_ply {
                            cb.call1(py, (json.as_str(),))?;
                        }
                        Ok(())
                    });
                    out.push(json);
                    match stop {
                        Ok(()) => Ok(true),
                        Err(e) => {
                            py_err = Some(e);
                            Ok(false)
                        }
                    }
                })
            });
            if let Some(e) = py_err {
                return Err(e);
            }
            res.map_err(engine_err)?;
            Ok(out)
        })
    }

    /// Objective best vs. practical best move at the end of `moves` (SAN, from `fen` or the
    /// start). Returns a Practical as JSON.
    #[pyo3(signature = (*, fen = None, moves = "", white_elo, black_elo, lambda_ = 0.3, tau = 0.7, horizon = 1, d = 0.5, p = 0.9, delta = 0.05, leaf = "calibrated", margin = 0.02))]
    #[allow(clippy::too_many_arguments)]
    fn practical(
        &self,
        py: Python<'_>,
        fen: Option<&str>,
        moves: &str,
        white_elo: f32,
        black_elo: f32,
        lambda_: f64,
        tau: f64,
        horizon: u32,
        d: f64,
        p: f64,
        delta: f64,
        leaf: &str,
        margin: f64,
    ) -> PyResult<String> {
        let leaf = match leaf {
            "calibrated" => LeafKind::Calibrated,
            "engine" => LeafKind::Engine,
            "maia" => LeafKind::Maia,
            other => return Err(PyValueError::new_err(format!("leaf must be 'calibrated', 'engine' or 'maia', not {other:?}"))),
        };
        if !(0.0..=1.0).contains(&lambda_) {
            return Err(PyValueError::new_err("lambda_ must be in [0, 1]"));
        }
        let hist = history_from(fen, moves).map_err(value_err)?;
        let params = PracticalParams { lambda: lambda_, tau, d, p, horizon, delta, leaf, margin, ..PracticalParams::default() };
        self.with_oracle(|o| {
            py.detach(|| {
                let r = practical(o, &hist, Ratings { white: white_elo, black: black_elo }, params)?;
                Ok(serde_json::to_string(&r)?)
            })
            .map_err(engine_err)
        })
    }

    /// Stop the Stockfish processes and free the model. Waits for running analyses. Calling
    /// it twice is harmless; using the engine afterwards raises EngineError.
    fn close(&self, py: Python<'_>) {
        py.detach(|| {
            if let Ok(mut g) = self.oracle.write() {
                g.take();
            }
        });
    }

    fn __repr__(&self) -> String {
        let open = self.oracle.read().map(|g| g.is_some()).unwrap_or(false);
        format!(
            "Engine(stockfish={:?}, maia={:?}, nodes={}{})",
            self.stockfish_name,
            self.maia_id,
            self.nodes,
            if open { "" } else { ", closed" }
        )
    }
}

fn game_json(index: usize, g: &anyhow::Result<Game>) -> serde_json::Value {
    match g {
        Ok(g) => {
            let moves: Vec<_> = g
                .moves
                .iter()
                .zip(&g.positions[1..])
                .map(|((san, uci), pos)| serde_json::json!({ "san": san, "uci": uci, "fen": full_fen(pos) }))
                .collect();
            serde_json::json!({
                "index": index,
                "tags": g.tags,
                "start_fen": full_fen(&g.positions[0]),
                "moves": moves,
                "error": null,
            })
        }
        Err(e) => serde_json::json!({
            "index": index, "tags": {}, "start_fen": null, "moves": [], "error": format!("{e:#}"),
        }),
    }
}

/// Every game in a PGN text, as a JSON list. A game that fails to parse keeps its place, with
/// `error` set.
#[pyfunction]
fn parse_pgn(text: &str) -> PyResult<String> {
    let games = parse_all(text).map_err(value_err)?;
    let list: Vec<_> = games.iter().enumerate().map(|(i, g)| game_json(i, g)).collect();
    Ok(serde_json::Value::Array(list).to_string())
}

#[pymodule]
mod _core {
    #[pymodule_export]
    use super::{parse_pgn, Engine, EngineError};

    #[pymodule_init]
    fn init(m: &pyo3::Bound<'_, pyo3::types::PyModule>) -> pyo3::PyResult<()> {
        use pyo3::types::PyModuleMethods;
        m.add("__version__", env!("CARGO_PKG_VERSION"))
    }
}
