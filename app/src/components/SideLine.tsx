import type { ReactNode } from "react";
import type { MoveInfo } from "../types";

interface Props {
  /** Half-moves played before the line starts (0 for the starting position). */
  basePly: number;
  /** e.g. "Side line from 24. Rxd4" or "Board from the starting position". */
  title: string;
  moves: MoveInfo[];
  cursor: number;
  onSelect: (i: number) => void;
  onClose: () => void;
  closeLabel: string;
}

/** The single side line being explored: numbered like the game, discarded on close. */
export function SideLine({ basePly, title, moves, cursor, onSelect, onClose, closeLabel }: Props) {
  const items: ReactNode[] = [];
  moves.forEach((m, i) => {
    const ply = basePly + i + 1; // half-move number of this move
    const white = ply % 2 === 1;
    if (white) items.push(<span className="n" key={`n${i}`}>{(ply + 1) / 2}.</span>);
    else if (i === 0) items.push(<span className="n" key={`n${i}`}>{ply / 2}...</span>);
    items.push(
      <span key={i} className={cursor === i + 1 ? "current" : ""} onClick={() => onSelect(i + 1)}>
        {m.san}
      </span>,
    );
  });
  return (
    <div className="panel sideline">
      <div className="head">
        <span className="title" onClick={() => onSelect(0)} style={{ cursor: "pointer" }}>
          {title}
        </span>
        <button onClick={onClose}>{closeLabel}</button>
      </div>
      {moves.length ? <div className="moves">{items}</div> : <div className="hint">Play a move on the board.</div>}
    </div>
  );
}
