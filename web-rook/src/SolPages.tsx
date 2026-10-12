import { SolCoins } from "./pages/SolCoins";
import { SolCoin } from "./pages/SolCoin";
import { SolLaunch } from "./pages/SolLaunch";
import { SolCollection, SolCollections } from "./pages/SolCollections";
import { SolTracked, TrackedOne } from "./pages/SolTracked";
import { SolReserves } from "./pages/SolReserves";
import { collectionBySlug } from "./rook/sol/collections";
import { NotOnSolana } from "./ui/sol";

/** The Solana pages (coins, a coin, launch, Rook's collections, the tracked-collection list), one lazy chunk. The wallet
 *  provider sits above, in App. `#/collection/<slug>` is a Rook collection's page for one of Rook's collections, a tracked collection's otherwise. */
export default function SolPages({ route }: { route: string[] }) {
  const [at, a] = route;
  return at === "coins" ? <SolCoins slug={a} /> : at === "coin" ? <SolCoin mint={a} /> : at === "launch" ? <SolLaunch slug={a} />
    : at === "collections" ? <SolCollections /> : at === "tracked" ? <SolTracked /> : at === "reserves" ? <SolReserves slug={a} />
    : at === "collection" ? (collectionBySlug(a) ? <SolCollection slug={a!} /> : <TrackedOne slug={a} />) : <NotOnSolana at={at} />;
}
