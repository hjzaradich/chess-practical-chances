// Mirrors the core's JSON (crates/core/src/analysis.rs). The GUI only displays these values.

export interface LikelyMove {
  san: string;
  uci: string;
  p: number;
  v: number;
  loses: boolean;
}

export interface Diagnostics {
  stable_moves: number;
  trap_mass: number;
  at_ply_offset: number;
  decision_fen: string;
  via: string | null;
  best_value: number;
  moves: LikelyMove[];
}

export interface Settings {
  k: number;
  p: number;
  nodes: number;
  d: number;
  epsilon: number;
  delta: number;
  multipv: number;
  white_elo: number;
  black_elo: number;
  stockfish: string;
  maia: string;
}

export interface Sides<T> {
  white: T;
  black: T;
}

export interface PlyResult {
  ply: number;
  san: string | null;
  uci: string | null;
  fen: string;
  side_to_move: "white" | "black";
  v_star: number;
  draw_rate: number;
  trickiness: Sides<number>;
  diagnostics: Sides<Diagnostics | null>;
  likely_moves: LikelyMove[];
  settings: Settings;
}

export interface MoveInfo {
  san: string;
  uci: string;
  fen: string;
}

export interface GameInfo {
  index: number;
  white: string;
  black: string;
  white_elo: number | null;
  black_elo: number | null;
  event: string;
  date: string;
  result: string;
  start_fen: string;
  moves: MoveInfo[];
  error: string | null;
}

export interface Status {
  model_path: string | null;
  stockfish_path: string | null;
  tablebases: string | null;
  data_dir: string;
  download_url: string | null;
  running: boolean;
}

export interface Done {
  status: "done" | "cancelled" | "error";
  message: string | null;
  from_saved: boolean;
}

export type Preset = "quick" | "standard" | "deep";

export const PRESETS: { id: Preset; label: string; detail: string }[] = [
  { id: "quick", label: "Quick", detail: "k=1 · 50k nodes · ~1 min/game" },
  { id: "standard", label: "Standard", detail: "k=2 · 50k nodes · ~3 min/game" },
  { id: "deep", label: "Deep", detail: "k=2 · 200k nodes · ~13 min/game" },
];

// Phase 4 (crates/core/src/practical.rs).
export interface Reply {
  san: string;
  uci: string;
  p: number;
  weight: number;
  objective: number;
  value: number;
  is_best: boolean;
  miss: boolean;
}

export interface Candidate {
  san: string;
  uci: string;
  p: number;
  engine_rank: number | null;
  objective: number;
  practical: number;
  best_reply: string | null;
  miss_chance: number;
  replies: Reply[];
}

export interface Practical {
  fen: string;
  side_to_move: "white" | "black";
  candidates: Candidate[];
  objective_best: number;
  practical_best: number;
  /** Q(practical best) − Q(objective best). */
  edge: number;
  /** A different move whose edge clears params.margin; otherwise there's no practical edge to show. */
  clear: boolean;
  params: { lambda: number; tau: number; d: number; p: number; horizon: number; delta: number; n_engine: number; n_human: number; leaf: string; margin: number };
  white_elo: number;
  black_elo: number;
  /** Calibrated leaf model used, if any (traceability). */
  leaf_model: string | null;
}
