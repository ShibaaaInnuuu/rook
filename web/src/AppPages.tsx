import Web3 from "./Web3";
import { Launch } from "./pages/Launch";
import { ListingPage } from "./pages/Listing";
import { Vaults } from "./pages/Vaults";
import { Explore } from "./pages/Explore";
import { Receipts } from "./pages/Receipts";
import { Claims } from "./pages/Claims";
import { Analytics } from "./pages/Analytics";
import { Draws } from "./pages/Draws";
import { Trust } from "./pages/Trust";
import { Docs } from "./pages/Docs";
import { Staging } from "./pages/Staging";
import { CollectionPage } from "./pages/Collection";
import { ReservesPage } from "./pages/Reserves";
import { RedeemPage } from "./pages/Redeem";
import { RentPage } from "./pages/Rent";
import { HandinsPage } from "./pages/Handins";
import { CoinRoute } from "./pages/CoinRoute";

/** Every page but the landing page, behind one lazy import: they are the ones that read contracts and can send a transaction. */
export default function AppPages({ route }: { route: string[] }) {
  const at = route[0];
  return (
    <Web3>
      {at === "launch" ? <Launch vault={route[1]} />
        : at === "coin" ? <CoinRoute address={route[1]} />
        : at === "listing" ? <ListingPage id={route[1]} />
        : at === "vaults" || at === "listings" ? <Vaults focus={route[1]} />
        : at === "explore" ? <Explore />
        : at === "receipts" ? <Receipts />
        : at === "claims" ? <Claims />
        : at === "analytics" ? <Analytics />
        : at === "draws" ? <Draws />
        : at === "docs" ? <Docs />
        : at === "staging" ? <Staging />
        : at === "collection" ? <CollectionPage slug={route[1]} />
        : at === "reserves" ? <ReservesPage slug={route[1]} />
        : at === "redeem" ? <RedeemPage slug={route[1]} ticket={route[2]} />
        : at === "rent" ? <RentPage slug={route[1]} />
        : at === "handins" ? <HandinsPage slug={route[1]} />
        : <Trust />}
    </Web3>
  );
}
