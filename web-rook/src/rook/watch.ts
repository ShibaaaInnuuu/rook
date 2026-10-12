import { useEffect, useState } from "react";

/**
 * The watchlist: starred coins, in this browser only (localStorage, `rook-watch`). Never required, never sent anywhere, and a
 * browser that refuses storage simply has no stars: every read and write is wrapped, and the page renders without it.
 */
const KEY = "rook-watch";
const readAll = (): string[] => { try { const v = JSON.parse(localStorage.getItem(KEY) ?? "[]"); return Array.isArray(v) ? v.filter((x) => typeof x === "string") : []; } catch { return []; } };
const listeners = new Set<() => void>();
function write(list: string[]) { try { localStorage.setItem(KEY, JSON.stringify(list)); } catch { /* no storage: the star does not stick, and nothing else changes */ } listeners.forEach((l) => l()); }

export function useWatchlist(): { list: string[]; has: (a: string) => boolean; toggle: (a: string) => void } {
  const [list, setList] = useState<string[]>(readAll);
  useEffect(() => { const l = () => setList(readAll()); listeners.add(l); addEventListener("storage", l); return () => { listeners.delete(l); removeEventListener("storage", l); }; }, []);
  const has = (a: string) => list.some((x) => x.toLowerCase() === a.toLowerCase());
  const toggle = (a: string) => write(has(a) ? list.filter((x) => x.toLowerCase() !== a.toLowerCase()) : [...list, a.toLowerCase()]);
  return { list, has, toggle };
}

/** A small local flag, for the first-visit panel and the like. Same rules: never required. */
export function useLocalFlag(key: string): [boolean, (v: boolean) => void] {
  const [v, setV] = useState<boolean>(() => { try { return localStorage.getItem(key) === "1"; } catch { return false; } });
  return [v, (x) => { setV(x); try { if (x) localStorage.setItem(key, "1"); else localStorage.removeItem(key); } catch { /* no storage */ } }];
}
