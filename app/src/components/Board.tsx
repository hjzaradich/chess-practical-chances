import { useEffect, useRef } from "react";
import { Chessground } from "chessground";
import type { Api } from "chessground/api";
import type { DrawShape } from "chessground/draw";
import type { Key } from "chessground/types";
import type { LikelyMove } from "../types";

interface Props {
  fen: string;
  lastMove: string | null; // uci
  orientation: "white" | "black";
  /** Likely moves for the side to move, drawn as arrows (red if they lose more than δ). */
  arrows: LikelyMove[];
  /** Legal moves (UCI, from the core). When given, the side to move can play them. */
  legal: string[];
  /** Called with origin and destination squares after a drag or click move. */
  onMove: (orig: string, dest: string) => void;
  /** Phase 4: the practical best move (UCI), drawn as a blue arrow. */
  practicalMove?: string | null;
}

const sq = (s: string) => s as Key;

/** chessground wants destinations per origin square; the moves themselves come from the core. */
function destsFrom(legal: string[]): Map<Key, Key[]> {
  const m = new Map<Key, Key[]>();
  for (const u of legal) {
    const o = sq(u.slice(0, 2));
    const d = sq(u.slice(2, 4));
    const list = m.get(o) ?? [];
    if (!list.includes(d)) list.push(d);
    m.set(o, list);
  }
  return m;
}

export function Board({ fen, lastMove, orientation, arrows, legal, onMove, practicalMove }: Props) {
  const el = useRef<HTMLDivElement>(null);
  const cg = useRef<Api | null>(null);
  const onMoveRef = useRef(onMove);
  onMoveRef.current = onMove;

  useEffect(() => {
    if (!el.current) return;
    cg.current = Chessground(el.current, {
      coordinates: true,
      animation: { enabled: true, duration: 150 },
      drawable: { enabled: false, visible: true },
      premovable: { enabled: false },
      movable: {
        free: false,
        showDests: true,
        events: { after: (orig, dest) => onMoveRef.current(orig, dest) },
      },
    });
    return () => cg.current?.destroy();
  }, []);

  useEffect(() => {
    const turn = fen.split(" ")[1] === "b" ? "black" : "white";
    const shapes: DrawShape[] = arrows.slice(0, 4).map((m) => ({
      orig: sq(m.uci.slice(0, 2)),
      dest: sq(m.uci.slice(2, 4)),
      brush: m.loses ? "red" : "green",
      modifiers: { lineWidth: Math.max(3, Math.round(4 + 12 * m.p)) },
    }));
    if (practicalMove) {
      shapes.push({ orig: sq(practicalMove.slice(0, 2)), dest: sq(practicalMove.slice(2, 4)), brush: "blue", modifiers: { lineWidth: 10 } });
    }
    cg.current?.set({
      fen: fen.split(" ")[0],
      orientation,
      turnColor: turn,
      check: false,
      lastMove: lastMove ? [sq(lastMove.slice(0, 2)), sq(lastMove.slice(2, 4))] : undefined,
      movable: { color: legal.length ? turn : undefined, dests: destsFrom(legal) },
      drawable: { autoShapes: shapes },
    });
  }, [fen, lastMove, orientation, arrows, legal, practicalMove]);

  return <div className="board-wrap"><div ref={el} className="cg-wrap" /></div>;
}

/** Choose the piece for a promotion. */
export function PromotionPicker({ color, onPick, onCancel }: { color: "white" | "black"; onPick: (p: string) => void; onCancel: () => void }) {
  const pieces: [string, string][] = color === "white"
    ? [["q", "♕"], ["r", "♖"], ["b", "♗"], ["n", "♘"]]
    : [["q", "♛"], ["r", "♜"], ["b", "♝"], ["n", "♞"]];
  return (
    <div className="overlay" onClick={onCancel}>
      <div className="dialog promo" onClick={(e) => e.stopPropagation()}>
        <h2>Promote to</h2>
        <div className="promo-row">
          {pieces.map(([p, glyph]) => (
            <button key={p} onClick={() => onPick(p)} aria-label={{ q: "Queen", r: "Rook", b: "Bishop", n: "Knight" }[p]}>
              {glyph}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}
