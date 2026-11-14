import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Done, GameInfo, MoveInfo, PlyResult, Practical, Preset, Status } from "./types";

export const api = {
  status: () => invoke<Status>("app_status"),
  modelDownloadSize: () => invoke<number>("model_download_size"),
  downloadModel: () => invoke<string>("download_model"),
  importModel: (path: string) => invoke<string>("import_model", { path }),
  parsePgn: (text: string) => invoke<GameInfo[]>("parse_pgn", { text }),
  readTextFile: (path: string) => invoke<string>("read_text_file", { path }),
  startAnalysis: (req: {
    pgn: string;
    game_index: number;
    white_elo: number;
    black_elo: number;
    preset: Preset;
  }) => invoke<void>("start_analysis", { req }),
  cancelAnalysis: () => invoke<void>("cancel_analysis"),
  exportResults: (path: string, format: "json" | "csv", plies: PlyResult[]) =>
    invoke<void>("export_results", { path, format, plies }),
  legalMoves: (fen: string) => invoke<string[]>("legal_moves", { fen }),
  playMove: (fen: string, uci: string) => invoke<MoveInfo>("play_move", { fen, uci }),
  analyzePosition: (req: {
    history: string[];
    ply: number;
    last_move: [string, string] | null;
    white_elo: number;
    black_elo: number;
    preset: Preset;
  }) => invoke<PlyResult>("analyze_position", { req }),
  analyzePractical: (req: { history: string[]; white_elo: number; black_elo: number; preset: Preset; d: number }) =>
    invoke<Practical>("analyze_practical", { req }),
};

export const events = {
  onPly: (f: (r: PlyResult) => void): Promise<UnlistenFn> => listen<PlyResult>("ply", (e) => f(e.payload)),
  onDone: (f: (d: Done) => void): Promise<UnlistenFn> => listen<Done>("analysis-done", (e) => f(e.payload)),
  onModelProgress: (f: (p: { done: number; total: number }) => void): Promise<UnlistenFn> =>
    listen<{ done: number; total: number }>("model-progress", (e) => f(e.payload)),
};

export function errorText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
}
