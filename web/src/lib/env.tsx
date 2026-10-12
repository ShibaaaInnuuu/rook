import { createContext, useContext, useEffect, useState, type ReactNode } from "react";
import { loadEnv, type Env } from "./chain";

const Ctx = createContext<Env | null>(null);
export function EnvProvider({ children }: { children: ReactNode }) {
  const [env, setEnv] = useState<Env | null>(null);
  useEffect(() => { void loadEnv().then(setEnv); }, []);
  return <Ctx.Provider value={env}>{children}</Ctx.Provider>;
}
/** null while loading; { mode: "none" } when there is no deployment to read. */
export const useEnv = () => useContext(Ctx);

export function useNow(): number {
  const [now, setNow] = useState(() => Math.floor(Date.now() / 1000));
  useEffect(() => { const t = setInterval(() => setNow(Math.floor(Date.now() / 1000)), 1000); return () => clearInterval(t); }, []);
  return now;
}

/** "#/coin/0x…" → ["coin", "0x…"]. A hash router: the site is static files and must work from any path. */
export function useRoute(): string[] {
  const read = () => location.hash.replace(/^#\/?/, "").split("/").filter(Boolean);
  const [route, setRoute] = useState(read);
  useEffect(() => { const on = () => { setRoute(read()); window.scrollTo(0, 0); }; addEventListener("hashchange", on); return () => removeEventListener("hashchange", on); }, []);
  return route;
}
export const href = (...parts: (string | number)[]) => `${location.search}#/${parts.join("/")}`;
