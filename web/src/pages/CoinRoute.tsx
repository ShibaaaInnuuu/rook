import { CoinPage } from "./Coin";
import { WrappedCoinPage } from "./WrappedCoin";
import { useWrappedCoin, useWrappedList, useWrappedSource } from "../lib/wrapped";
import { FixtureBanner } from "../components/wx";
import { Loading } from "../components/ui";

/** `#/coin/<address>`: a coin launched on a wrapped collection (docs/v5/ux-and-reserves.md) gets the one-click page; any other is the Pons coin page as before. */
export function CoinRoute({ address }: { address?: string }) {
  const src = useWrappedSource(), list = useWrappedList(), found = useWrappedCoin(address);
  // wait for the answer, unless the collections could not be read at all: then this is the ordinary coin page
  if (src.isLoading || (src.data && !list.error && !found.error && found.isPending)) return <Loading what="the coin" />;
  if (src.data && found.data) return <>{src.data.kind === "fixture" ? <FixtureBanner note={src.data.note} /> : null}<WrappedCoinPage collection={found.data.collection} coin={found.data.coin} source={src.data} /></>;
  return <CoinPage address={address} />;
}
