import { useState } from "react";
import { keccak256, toHex } from "viem";
import { useAccount } from "wagmi";
import { useEnv } from "../lib/env";
import { useLaunchFee, useListings } from "../lib/data";
import { LAUNCHER } from "../lib/abis";
import { Failed, Loading, NoDeployment } from "../components/ui";
import { PageHead } from "../components/catalog";
import { VaultChooser } from "../components/catalogue";
import { useCatalogue } from "../lib/catalogue";
import { TxButton } from "../components/wallet";
import { eth } from "../lib/format";
import { MEASURED_BASE_TAKE_BPS, VAULT_SHARE_BPS } from "../lib/estimate";

const STEPS = ["The coin", "Its vault", "Who gets it", "Review"] as const;
type Step = 0 | 1 | 2 | 3;

/** The creator's one power, met at launch rather than in an email later. It is recorded per acquisition, not per coin, so this
 *  is a stated intention and can be decided again for each asset. */
const POLICIES: [string, string, string][] = [
  ["draw-weighted", "A draw, tickets by balance", "Every holder is entered in proportion to what they hold. One wallet wins each inscription."],
  ["draw-per-wallet", "A draw, one ticket a wallet", "Every holder is entered once, however much they hold. Flatter odds, and easier to work with many wallets."],
  ["pro-rata", "Shared out pro rata", "For a rune: divided among holders by balance, each claiming with a proof. An inscription cannot be divided, so this is not offered for one."],
  ["hold", "Held in the vault", "The asset stays where it is. Nobody receives it until you choose otherwise."],
];

const Figure = ({ label, value, unit }: { label: string; value: string; unit: string }) => (
  <div className="figure"><dt>{label}</dt><dd><span className="num">{value}</span><span className="unit"> {unit}</span></dd></div>
);

/**
 * `#/launch/<vault id>` opens with that vault already chosen. The vault's own page links here (docs/phase14.md §8.6), and a
 * reader who arrived from one vault's page and had to find it again in a list of sixty would rightly wonder what the link
 * was for. An id that names no vault is ignored: the chooser opens empty rather than on somebody else's vault.
 */
export function Launch({ vault }: { vault?: string } = {}) {
  const env = useEnv();
  const listings = useListings();
  const catalogue = useCatalogue();
  const { address } = useAccount();
  const fee = useLaunchFee();
  const [step, setStep] = useState<Step>(0);
  const [form, setForm] = useState({ name: "", symbol: "", tax: "2", listing: /^\d{1,6}$/.test(vault ?? "") ? vault! : "", policy: "draw-weighted" });
  const live = env && env.mode !== "none" ? env : null;

  const tax = Number(form.tax);
  const chosen = listings.data?.find((l) => String(l.id) === form.listing);
  const set = (k: keyof typeof form) => (e: { target: { value: string } }) => setForm((f) => ({ ...f, [k]: e.target.value }));

  const stepProblem: (string | null)[] = [
    !form.name.trim() ? "Give the coin a name." : !/^[A-Za-z0-9]{2,10}$/.test(form.symbol) ? "A symbol is 2 to 10 letters or digits." : !(tax >= 0 && tax <= 10 && Number.isFinite(tax)) ? "The creator tax is between 0 and 10 percent." : null,
    form.listing === "" ? "Choose the vault this coin's fees pay into." : null,
    null,
    null,
  ];
  const problem = stepProblem.slice(0, step + 1).find((p) => p) ?? null;

  // what 1 ETH of trading sends to the vault, at the measured rate (docs/phase1.md)
  const takeBps = MEASURED_BASE_TAKE_BPS + BigInt(Math.round((Number.isFinite(tax) ? Math.min(Math.max(tax, 0), 10) : 0) * 100));
  const perEthToVault = (10n ** 18n * takeBps * VAULT_SHARE_BPS) / (10_000n * 10_000n);
  const perEthToTreasury = (10n ** 18n * takeBps * (10_000n - VAULT_SHARE_BPS)) / (10_000n * 10_000n);

  return (
    <>
      <PageHead title="Launch a coin">
        <p>One transaction on Pons, through the Launcher. The pairing you choose here cannot be changed afterwards by you, by us, or by the coin's holders.</p>
      </PageHead>

      {env === null ? <Loading what="the deployment" /> : !live ? (
        <><p className="banner banner--block" role="status">Launching is switched off: there is no Rook deployment on this chain for a launch to go through.</p><NoDeployment /></>
      ) : (
        <div className="wizard">
          <div className="wizard__main">
            <ol className="steps-bar">
              {STEPS.map((s, i) => (
                <li key={s} aria-current={i === step ? "step" : undefined}>
                  <button type="button" className="steps-bar__go" onClick={() => setStep(i as Step)} disabled={i > step && !!problem}>{s}</button>
                </li>
              ))}
            </ol>

            <form className="form" onSubmit={(e) => e.preventDefault()} noValidate>
              {step === 0 ? (
                <>
                  <label htmlFor="l-name">Name<input id="l-name" value={form.name} onChange={set("name")} maxLength={40} autoComplete="off" /></label>
                  <label htmlFor="l-symbol">Symbol<input id="l-symbol" value={form.symbol} onChange={(e) => set("symbol")({ target: { value: e.target.value.toUpperCase() } })} maxLength={10} autoComplete="off" className="mono" /></label>
                  <label htmlFor="l-tax">Creator tax, percent
                    <input id="l-tax" type="number" min={0} max={10} step={0.5} value={form.tax} onChange={set("tax")} inputMode="decimal" />
                    <small>On top of Pons's base fee. All of it goes to the router: none of it, ever, to you. A higher tax fills the vault with less trading and makes the coin dearer to trade.</small>
                  </label>
                </>
              ) : step === 1 ? (
                <>
                  <p className="wizard__ask">The vault this coin's fees pay into</p>
                  <p className="quiet">Permanent. The router is created naming this vault and can never be pointed anywhere else.</p>
                  {listings.isLoading ? <Loading what="vaults" /> : (
                    <VaultChooser
                      vaults={listings.data ?? []}
                      value={form.listing}
                      onPick={(id) => setForm((f) => ({ ...f, listing: id }))}
                      catalogue={catalogue.data}
                    />
                  )}
                  {listings.error ? <Failed what="vaults" error={listings.error} /> : null}
                  {chosen ? <p className="quiet">Fees reach the <strong>{chosen.name}</strong> vault, which can buy that {chosen.kind === "rune" ? "rune" : "collection"} and nothing else. Other coins may pay into the same vault.</p> : null}
                </>
              ) : step === 2 ? (
                <fieldset className="choices">
                  <legend>What happens to what it buys</legend>
                  <p className="quiet">Your one power as creator. It is recorded for each asset your coin pays for, so it can be decided again later; this is what you intend today.</p>
                  {POLICIES.filter(([id]) => id !== "pro-rata" || chosen?.kind === "rune").map(([id, title, note]) => (
                    <label key={id} className={`choice${form.policy === id ? " choice--on" : ""}`}>
                      <input type="radio" name="policy" value={id} checked={form.policy === id} onChange={set("policy")} />
                      <span><strong>{title}</strong><small>{note}</small></span>
                    </label>
                  ))}
                </fieldset>
              ) : (
                <div className="stack">
                  <dl className="kv kv--wide">
                    <dt>Coin</dt><dd>{form.name} <span className="mono quiet">{form.symbol}</span></dd>
                    <dt>Creator tax</dt><dd><span className="num">{tax}</span>%, all of it to the router</dd>
                    <dt>Buys</dt><dd>{chosen ? `${chosen.name}, ${chosen.kind === "rune" ? "a rune" : "inscriptions"}` : "not chosen"}</dd>
                    <dt>Intended for</dt><dd>{POLICIES.find(([id]) => id === form.policy)?.[1] ?? "not chosen"}</dd>
                    <dt>Launch fee</dt><dd>{fee.data !== undefined ? <><span className="num">{eth(fee.data, 5)}</span> ETH, Pons's own</> : "not read yet"}</dd>
                  </dl>
                  <ol className="steps">
                    <li>Pays Pons's launch fee and creates the coin and its bonding curve on Pons V2.</li>
                    <li>Creates the coin's router, which has no owner, and names it to Pons as the creator-fee recipient. It sends 80% of what it receives to {chosen ? <strong>{chosen.name}</strong> : "the listing"}'s vault and 20% to the treasury.</li>
                    <li>Records you as the creator, which gives you the choice above and nothing else: no fee, none of the supply, and no way to change the pairing.</li>
                  </ol>
                  <p className="quiet">Pons's own protocol owner can redirect any coin's creator fees after a three-day public timelock. If that happened, fees would stop arriving and the vault would keep what it already holds.</p>
                  <TxButton label={fee.data !== undefined ? `Launch for ${eth(fee.data, 5)} ETH` : "Launch"} doing="Launching…"
                    disabledWhy={problem ?? (fee.data === undefined ? "Pons's launch fee has not been read, so the transaction cannot be built." : null)}
                    request={() => ({ address: live.deployment.launcher, abi: LAUNCHER, functionName: "launch", value: fee.data!, args: [{ name: form.name.trim(), symbol: form.symbol, logo: "", description: "",
                      socials: { twitter: "", telegram: "", discord: "", website: "", farcaster: "" }, creatorTaxBps: Math.round(tax * 100), expectedEconomics: `0x${"00".repeat(32)}`,
                      salt: keccak256(toHex(`${address ?? ""}:${form.symbol}:${Date.now()}`)), launchConfigId: 0n, listingId: BigInt(form.listing) }] })} />
                </div>
              )}

              <div className="actions actions--start">
                {step > 0 ? <button type="button" className="btn btn--quiet" onClick={() => setStep((step - 1) as Step)}>Back</button> : null}
                {step < 3 ? <><button type="button" className="btn" disabled={!!stepProblem[step]} onClick={() => setStep((step + 1) as Step)}>Next</button>
                  {stepProblem[step] ? <span className="quiet">{stepProblem[step]}</span> : null}</> : null}
              </div>
            </form>
          </div>

          <aside className="wizard__side" aria-label="What this choice sends">
            <h2>Per 1 ETH traded</h2>
            <dl className="figures figures--tight">
              <Figure label="to the vault" value={eth(perEthToVault, 5)} unit="ETH" />
              <Figure label="to the treasury" value={eth(perEthToTreasury, 5)} unit="ETH" />
              <Figure label="to you" value="0" unit="ETH" />
            </dl>
            <div className="split-bar" role="img" aria-label="80 percent to the vault, 20 percent to the treasury"><div>80</div><div>20</div></div>
            <p className="formula mono">(0.70% + {(Number.isFinite(tax) ? Math.min(Math.max(tax, 0), 10) : 0).toFixed(2)}% tax) × 80%</p>
            <p className="quiet">An estimate. 0.70% is what was measured reaching a router per unit of volume, on the curve and in the pool alike, on a fork and against eight real graduated coins. It assumes Pons's operator keeps sweeping pool fees after graduation.</p>
          </aside>
        </div>
      )}
    </>
  );
}
