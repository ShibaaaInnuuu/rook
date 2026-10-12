import "@fontsource/tenor-sans/latin-400.css";
import "@fontsource-variable/hanken-grotesk/wght.css";
import "@fontsource/ibm-plex-mono/latin-400.css";
import "@fontsource/ibm-plex-mono/latin-500.css";
import "./styles/etch.css";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { App } from "./App";

// The landing page needs neither viem nor wagmi. The wallet, and every page that can send a transaction, load as one lazy chunk
// (src/Web3.tsx); the chain client is imported only when there is a deployment to read (src/lib/chain.ts).
const queries = new QueryClient({ defaultOptions: { queries: { refetchInterval: 15_000, staleTime: 5_000, retry: 1 } } });

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <QueryClientProvider client={queries}>
      <App />
    </QueryClientProvider>
  </StrictMode>,
);
