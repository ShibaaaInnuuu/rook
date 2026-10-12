import { useEffect } from "react";
import { useAccount, useConnect } from "wagmi";
import Web3 from "./Web3";
import { Wallet } from "./components/wallet";

/** The pressed "Connect wallet" button is what loaded this chunk, so the press is carried through: connect once, on arrival. */
function ConnectOnArrival({ now }: { now: boolean }) {
  const { isConnected } = useAccount(); const { connect, connectors } = useConnect();
  useEffect(() => { if (now && !isConnected && connectors[0]) connect({ connector: connectors[0] }); }, []);      // eslint-disable-line react-hooks/exhaustive-deps
  return null;
}
export default function WalletIsland({ connectNow = false }: { connectNow?: boolean }) { return <Web3><ConnectOnArrival now={connectNow} /><Wallet /></Web3>; }
