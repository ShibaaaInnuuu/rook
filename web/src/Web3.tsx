import type { ReactNode } from "react";
import { WagmiProvider, createConfig, http } from "wagmi";
import { injected } from "@wagmi/core";      // not "wagmi/connectors": that barrel drags in MetaMask's deprecated SDK
import { local, robinhood, stagingChain } from "./lib/chains";
import { STAGING_RPC } from "./lib/chain";

// One connector: whatever wallet the browser has. No WalletConnect relay, no third-party request of any kind.
// the staging fork is offered to a wallet only in a build that was told where it is
const staging = STAGING_RPC ? stagingChain(STAGING_RPC) : null;
const wagmi = staging
  ? createConfig({ chains: [robinhood, local, staging], connectors: [injected()], transports: { [robinhood.id]: http(), [local.id]: http(), [staging.id]: http(STAGING_RPC!) } })
  : createConfig({ chains: [robinhood, local], connectors: [injected()], transports: { [robinhood.id]: http(), [local.id]: http() } });
export default function Web3({ children }: { children: ReactNode }) { return <WagmiProvider config={wagmi}>{children}</WagmiProvider>; }
