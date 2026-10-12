import Web3 from "./Web3";
import { Coins } from "./pages/Coins";
import { CoinPage } from "./pages/Coin";
import { Launch } from "./pages/Launch";
import { Collections, CollectionPage } from "./pages/Collection";
import { ReservesPage } from "./pages/Reserves";
import { RedeemPage } from "./pages/Redeem";
import { DrawPage } from "./pages/Draw";
import { RentPage } from "./pages/Rent";
import { Bots } from "./pages/Bots";
import { WalletPage } from "./pages/Wallet";

/** The Robinhood Chain design's pages, behind one lazy chunk with the wagmi provider. UNROUTED on the Solana site: `#/tracked` and
 *  `#/collection/<slug>` for a slug outside Rook's collections are src/pages/SolTracked.tsx's now (no wagmi, no indexer needed). Kept in
 *  the tree with the other Robinhood pages. */
export default function AppPages({ route }: { route: string[] }) {
  const [at, a, b] = route;
  return (
    <Web3>
      {at === "coins" ? <Coins /> : at === "coin" ? <CoinPage address={a} /> : at === "launch" ? <Launch slug={a} />
        : at === "collections" || at === "tracked" ? <Collections /> : at === "collection" ? <CollectionPage slug={a} />
        : at === "reserves" ? <ReservesPage slug={a} /> : at === "redeem" ? <RedeemPage slug={a} ticket={b} /> : at === "draw" ? <DrawPage slug={a} ticket={b} /> : at === "rent" ? <RentPage slug={a} /> : at === "wallet" ? <WalletPage address={a} />
        : <Bots slug={a} />}
    </Web3>
  );
}
