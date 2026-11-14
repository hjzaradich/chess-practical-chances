import { useEffect, useRef } from "react";
import type { MoveInfo, PlyResult } from "../types";

interface Props {
  moves: MoveInfo[];
  plies: (PlyResult | undefined)[];
  current: number;
  onSelect: (ply: number) => void;
}

/** A small dot after a move when the player who made it faced high T there (≥ 0.2). */
function tick(r: PlyResult | undefined) {
  if (!r) return null;
  const t = r.side_to_move === "white" ? r.trickiness.white : r.trickiness.black;
  if (t < 0.2) return null;
  const color = r.side_to_move === "white" ? "var(--series-3)" : "var(--series-4)";
  return <span className="tick" style={{ background: color }} title={`T for the mover here: ${t.toFixed(2)}`} />;
}

export function MoveList({ moves, plies, current, onSelect }: Props) {
  const box = useRef<HTMLDivElement>(null);

  useEffect(() => {
    box.current?.querySelector(".current")?.scrollIntoView({ block: "nearest" });
  }, [current]);

  const rows = [];
  for (let i = 0; i < moves.length; i += 2) {
    const cell = (ply: number) =>
      ply <= moves.length ? (
        <div className={`mv${current === ply ? " current" : ""}`} onClick={() => onSelect(ply)}>
          <span>{moves[ply - 1].san}</span>
          {tick(plies[ply - 1])}
        </div>
      ) : (
        <div />
      );
    rows.push(
      <div className="row" key={i}>
        <span className="num">{i / 2 + 1}.</span>
        {cell(i + 1)}
        {cell(i + 2)}
      </div>,
    );
  }
  return (
    <div className="panel movelist" ref={box}>
      <div className={`start${current === 0 ? " current" : ""}`} onClick={() => onSelect(0)}>
        Start
      </div>
      {rows}
    </div>
  );
}
