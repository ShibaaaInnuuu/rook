import { useEffect, useRef, useState } from "react";
import { href } from "../lib/env";
import { searchRows } from "../rook/boardlogic";
import { useBoard } from "../rook/data";
import { CoinFace } from "./Identicon";

/** The masthead's search: instant results over the board's rows by name, symbol, nest or address; a full address opens the wallet page. */
export function Search() {
  const board = useBoard();
  const [q, setQ] = useState("");
  const [open, setOpen] = useState(false);
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => { const on = (e: MouseEvent) => { if (box.current && !box.current.contains(e.target as Node)) setOpen(false); }; addEventListener("mousedown", on); return () => removeEventListener("mousedown", on); }, []);
  const rows = q.trim() ? searchRows(board.data?.rows ?? [], q).slice(0, 8) : [];
  const isAddr = /^0x[0-9a-fA-F]{40}$/.test(q.trim());
  const go = () => { if (isAddr && !rows.length) { location.hash = `#/wallet/${q.trim()}`; setOpen(false); setQ(""); } else if (rows[0]) { location.hash = `#/coin/${rows[0].address}`; setOpen(false); setQ(""); } };
  return (
    <div className="search" ref={box}>
      <input type="search" value={q} placeholder="Search coins or paste an address" aria-label="Search coins or wallets" autoComplete="off" spellCheck={false}
        onChange={(e) => { setQ(e.target.value); setOpen(true); }} onFocus={() => setOpen(true)} onKeyDown={(e) => { if (e.key === "Enter") go(); if (e.key === "Escape") setOpen(false); }} />
      {open && q.trim() ? (
        <ul className="search__results" role="listbox">
          {rows.map((r) => <li key={r.address}><a href={href("coin", r.address)} onClick={() => { setOpen(false); setQ(""); }}><CoinFace address={r.address} logoUrl={r.logoUrl} name={r.name} size={24} /><strong>{r.symbol}</strong><span className="muted">{r.name} · {r.nest.name}</span></a></li>)}
          {isAddr ? <li><a href={href("wallet", q.trim())} onClick={() => { setOpen(false); setQ(""); }}><span className="mono">{q.trim().slice(0, 10)}…</span><span className="muted">open this wallet</span></a></li> : null}
          {!rows.length && !isAddr ? <li className="muted small" style={{ padding: ".5rem .75rem" }}>{board.data ? "Nothing matches." : "The board is not loaded."}</li> : null}
        </ul>) : null}
    </div>
  );
}
