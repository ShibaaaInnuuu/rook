/**
 * Solana wallets through the Wallet Standard (Phantom, Solflare, Backpack and any other wallet that registers itself): discovery with
 * @wallet-standard/app's getWallets(), then the wallet's own `standard:connect`, `solana:signTransaction` and
 * `solana:signAndSendTransaction` features. No adapter library, no WalletConnect relay, no third-party request: the wallet is the
 * browser extension (or the wallet's in-app browser) itself. The last wallet used is remembered in this browser only.
 */
import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import { getWallets } from "@wallet-standard/app";
import type { Wallet, WalletAccount } from "@wallet-standard/base";
import { b58encode } from "./base58";

export const SOLANA_CHAIN = "solana:mainnet";
type ConnectFeature = { connect: (input?: { silent?: boolean }) => Promise<{ accounts: readonly WalletAccount[] }> };
type DisconnectFeature = { disconnect: () => Promise<void> };
type EventsFeature = { on: (event: "change", listener: (p: { accounts?: readonly WalletAccount[] }) => void) => () => void };
type SignTxFeature = { signTransaction: (...inputs: { account: WalletAccount; transaction: Uint8Array; chain?: string }[]) => Promise<readonly { signedTransaction: Uint8Array }[]> };
type SignSendFeature = { signAndSendTransaction: (...inputs: { account: WalletAccount; transaction: Uint8Array; chain: string; options?: { commitment?: string; skipPreflight?: boolean; maxRetries?: number } }[]) => Promise<readonly { signature: Uint8Array }[]> };
const feat = <T,>(w: Wallet, name: string): T | null => ((w.features as Record<string, unknown>)[name] as T | undefined) ?? null;

/** A wallet the site can use: it connects, it is on Solana mainnet, and it can sign a transaction. */
export const usable = (w: Wallet) => !!feat(w, "standard:connect") && w.chains.some((c) => c === SOLANA_CHAIN) && (!!feat(w, "solana:signTransaction") || !!feat(w, "solana:signAndSendTransaction"));

export type SolWallet = {
  wallets: readonly Wallet[];
  wallet: Wallet | null; account: WalletAccount | null; address: string | null;
  connecting: boolean; error: string | null;
  connect: (w: Wallet) => Promise<void>; disconnect: () => Promise<void>;
  /** The wallet signs (and may change the priority fee of) a serialized transaction; returns the signed wire bytes. */
  signTransaction: ((tx: Uint8Array) => Promise<Uint8Array>) | null;
  /** The wallet signs and sends; returns the signature (base58). */
  signAndSend: ((tx: Uint8Array) => Promise<string>) | null;
};
const Ctx = createContext<SolWallet | null>(null);
const KEY = "rook-sol-wallet";
const remember = (name: string | null) => { try { if (name) localStorage.setItem(KEY, name); else localStorage.removeItem(KEY); } catch { /* no storage */ } };
const remembered = () => { try { return localStorage.getItem(KEY); } catch { return null; } };

export function SolWalletProvider({ children }: { children: ReactNode }) {
  const [wallets, setWallets] = useState<readonly Wallet[]>([]);
  const [wallet, setWallet] = useState<Wallet | null>(null);
  const [account, setAccount] = useState<WalletAccount | null>(null);
  const [connecting, setConnecting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const api = getWallets();
    const refresh = () => setWallets(api.get().filter(usable));
    refresh();
    const offs = [api.on("register", refresh), api.on("unregister", refresh)];
    return () => offs.forEach((f) => f());
  }, []);

  // a returning visitor: reconnect silently to the wallet used last (the wallet decides whether that needs a prompt)
  useEffect(() => {
    if (wallet) return;
    const name = remembered();
    const w = name ? wallets.find((x) => x.name === name) : null;
    if (!w) return;
    void feat<ConnectFeature>(w, "standard:connect")!.connect({ silent: true }).then((r) => {
      const a = r.accounts.find((x) => x.chains.includes(SOLANA_CHAIN)) ?? r.accounts[0];
      if (a) { setWallet(w); setAccount(a); }
    }).catch(() => { /* silent: stay disconnected */ });
  }, [wallets, wallet]);

  useEffect(() => {
    if (!wallet) return;
    const ev = feat<EventsFeature>(wallet, "standard:events");
    if (!ev) return;
    return ev.on("change", (p) => { if (p.accounts) setAccount(p.accounts[0] ?? null); });
  }, [wallet]);

  const connect = useCallback(async (w: Wallet) => {
    setConnecting(true); setError(null);
    try {
      const r = await feat<ConnectFeature>(w, "standard:connect")!.connect();
      const a = r.accounts.find((x) => x.chains.includes(SOLANA_CHAIN)) ?? r.accounts[0];
      if (!a) throw new Error(`${w.name} returned no Solana account`);
      setWallet(w); setAccount(a); remember(w.name);
    } catch (e) {
      const m = String((e as Error)?.message ?? e);
      setError(/reject|denied|cancel/i.test(m) ? "You declined in your wallet." : m.slice(0, 200));
    } finally { setConnecting(false); }
  }, []);
  const disconnect = useCallback(async () => {
    try { await feat<DisconnectFeature>(wallet!, "standard:disconnect")?.disconnect(); } catch { /* the wallet may not support it */ }
    setWallet(null); setAccount(null); remember(null);
  }, [wallet]);

  const value = useMemo<SolWallet>(() => {
    const st = wallet && account ? feat<SignTxFeature>(wallet, "solana:signTransaction") : null;
    const ss = wallet && account ? feat<SignSendFeature>(wallet, "solana:signAndSendTransaction") : null;
    return {
      wallets, wallet, account, address: account?.address ?? null, connecting, error, connect, disconnect,
      signTransaction: st ? async (tx) => (await st.signTransaction({ account: account!, transaction: tx, chain: SOLANA_CHAIN }))[0].signedTransaction : null,
      signAndSend: ss ? async (tx) => b58encode((await ss.signAndSendTransaction({ account: account!, transaction: tx, chain: SOLANA_CHAIN, options: { commitment: "confirmed" } }))[0].signature) : null,
    };
  }, [wallets, wallet, account, connecting, error, connect, disconnect]);
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useSolWallet(): SolWallet {
  const v = useContext(Ctx);
  if (!v) throw new Error("useSolWallet outside SolWalletProvider");
  return v;
}

const short = (a: string) => `${a.slice(0, 4)}…${a.slice(-4)}`;
/** The masthead's button: connect (a menu of the wallets this browser has), or the connected address (press to disconnect). */
export function SolWalletButton({ openNow = false }: { openNow?: boolean }) {
  const w = useSolWallet();
  const [open, setOpen] = useState(openNow);
  if (w.address) return <button className="btn btn--quiet btn--small" onClick={() => void w.disconnect()} title={`${w.wallet?.name}: ${w.address}. Press to disconnect.`}><span className="mono">{short(w.address)}</span></button>;
  return (
    <span className="walletmenu">
      <button className="btn btn--quiet btn--small" aria-expanded={open} disabled={w.connecting} onClick={() => setOpen(!open)}>{w.connecting ? "Connecting…" : "Connect wallet"}</button>
      {open ? (
        <span className="walletmenu__list panel" role="menu">
          {w.wallets.length ? w.wallets.map((x) => (
            <button key={x.name} type="button" role="menuitem" className="walletmenu__item" onClick={() => { setOpen(false); void w.connect(x); }}>
              {x.icon ? <img src={x.icon} alt="" width={20} height={20} /> : null}{x.name}
            </button>
          )) : <span className="small muted walletmenu__none">No Solana wallet found in this browser. Install <a href="https://phantom.com" target="_blank" rel="noreferrer noopener">Phantom</a>, <a href="https://solflare.com" target="_blank" rel="noreferrer noopener">Solflare</a> or <a href="https://backpack.app" target="_blank" rel="noreferrer noopener">Backpack</a>, or open this site in the wallet's own browser.</span>}
          {w.error ? <span className="small error walletmenu__none">{w.error}</span> : null}
        </span>
      ) : null}
    </span>
  );
}
