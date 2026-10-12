import { useEnv } from "../lib/env";
import { useBoard } from "../rook/data";
import { Board } from "../ui/Board";
import { FixtureBanner, Loading, NothingDeployed } from "../ui/bits";

/** The board alone, under #/coins: the same rows as the home page without the strip. */
export function Coins() {
  const env = useEnv(), board = useBoard();
  if (env !== null && !board.sourceLoading && !board.source) return <NothingDeployed page="coins" />;
  return (
    <>
      {board.source?.kind === "fixture" ? <FixtureBanner note={board.source.note} /> : null}
      <header className="pagehead rise"><p className="eyebrow">Coins</p><h1>Every coin on the board</h1></header>
      {board.isLoading || board.sourceLoading ? <Loading what="the board" /> : board.data ? <div className="rise rise-1"><Board rows={board.data.rows} usdPerEth={board.data.usdPerEth} initialTab="all" /></div> : null}
    </>
  );
}
