/**
 * Each Rook collection's official links, keyed by slug (src/rook/sol/collections.ts): its X account, its website when it has one,
 * and where it trades. The X handles and websites are the operator's verified list (2026-10-10). Marketplace pages:
 *  - Ethereum: OpenSea, at deployments/launch/nft-pilot.json's `openseaSlug`; Robinhood Chain: OpenSea, at robinhood-collections.json's;
 *  - Ordinals: Ordinals Wallet, at the `owSlug` its members provenance file was read from (deployments/launch/btc/*.provenance.json;
 *    Bitcoin Frogs: nft-pilot.json `bitcoin.owSlug`), and Magic Eden only where its collection symbol was confirmed against Magic
 *    Eden's own API (Bitcoin Frogs, Bitcoin Wizards).
 * test/sol.test.ts holds one entry per collection, https only.
 */
export type Marketplace = { name: "OpenSea" | "Ordinals Wallet" | "Magic Eden"; url: string };
export type CollectionLinks = {
  /** the X handle, without the @ (none where no verified handle is recorded: the Robinhood Chain collections) */
  x?: string;
  /** whose account it is, when it is not the collection's own */
  xNote?: string;
  website?: string;
  marketplaces: Marketplace[];
};

const os = (slug: string): Marketplace => ({ name: "OpenSea", url: `https://opensea.io/collection/${slug}` });
const ow = (slug: string): Marketplace => ({ name: "Ordinals Wallet", url: `https://ordinalswallet.com/collection/${slug}` });
const me = (slug: string): Marketplace => ({ name: "Magic Eden", url: `https://magiceden.io/ordinals/marketplace/${slug}` });
const meSol = (symbol: string): Marketplace => ({ name: "Magic Eden", url: `https://magiceden.io/marketplace/${symbol}` });

export const COLLECTION_LINKS: Readonly<Record<string, CollectionLinks>> = {
  milady: { x: "MiladyMaker333", website: "https://miladymaker.net", marketplaces: [os("milady")] },
  azuki: { x: "Azuki", website: "https://azuki.com", marketplaces: [os("azuki")] },
  mayc: { x: "BoredApeYC", xNote: "Yuga Labs' account, shared with BAYC", marketplaces: [os("mutant-ape-yacht-club")] },
  pudgypenguins: { x: "pudgypenguins", marketplaces: [os("pudgypenguins")] },
  bayc: { x: "BoredApeYC", marketplaces: [os("boredapeyachtclub")] },
  doodles: { x: "doodles", marketplaces: [os("doodles-official")] },
  moonbirds: { x: "moonbirds", website: "https://moonbirds.com", marketplaces: [os("moonbirds")] },
  clonex: { x: "RTFKT", marketplaces: [os("clonex")] },
  lilpudgys: { x: "pudgypenguins", xNote: "Pudgy Penguins' account", marketplaces: [os("lilpudgys")] },
  checks: { x: "jackbutcher", xNote: "the artist, Jack Butcher", website: "https://checks.art", marketplaces: [os("vv-checks-originals")] },
  identitymd: { x: "surfcoderepeat", xNote: "the creator", website: "https://imd.fun", marketplaces: [os("identitymd")] },
  "bitcoin-frogs": { x: "BitcoinFrogs", website: "https://bitcoinfrogs.com", marketplaces: [ow("bitcoin-frogs"), me("bitcoin-frogs")] },
  nodemonkes: { x: "nodemonkes", website: "https://nodemonkes.com", marketplaces: [ow("nodemonkes")] },
  omb: { x: "OrdinalMaxiBiz", website: "https://ordinalmaxibiz.wiki", marketplaces: [ow("omb")] },
  "bitcoin-puppets": { x: "BitcoinPuppets", marketplaces: [ow("bitcoin-puppets")] },
  "bitcoin-punks": { x: "Bitcoin_Punks_", website: "https://bitcoinpunks.com", marketplaces: [ow("bitcoinpunks")] },
  natcats: { x: "dmtnatcats", marketplaces: [ow("dmtnatcats")] },
  runestone: { x: "LeonidasNFT", xNote: "Leonidas, who made it", marketplaces: [ow("runestone")] },
  "bitcoin-weirdos": { x: "ftw_collective", website: "https://f-t-w.xyz", marketplaces: [ow("bitcoin-weirdos")] },
  cents: { x: "sovrnart", website: "https://sovrn.art/cents", marketplaces: [ow("cents")] },
  rsic: { x: "rune_coin", website: "https://runecoin.io", marketplaces: [ow("rsic")] },
  "pizza-ninjas": { x: "ninjalerts", marketplaces: [ow("pizza-ninjas")] },
  "quantum-cats": { x: "TaprootWizards", xNote: "Taproot Wizards' account", website: "https://quantumcats.xyz", marketplaces: [ow("quantum_cats")] },
  "wizards-of-ord": { x: "TheWizardsOfOrd", website: "https://wizards.art", marketplaces: [ow("wizards")] },
  "taproot-wizards": { x: "TaprootWizards", website: "https://taprootwizards.com", marketplaces: [ow("taproot-wizards")] },
  "bitcoin-wizards": { x: "bitcoinwizardry", website: "https://bitcoinwizard.com", marketplaces: [ow("bitcoin-wizards"), me("bitcoin-wizards")] },
  // the Solana collections: X and website as deployments/launch/solana-collections.json verified them, Magic Eden at its `magic_eden_symbol`
  mad_lads: { x: "MadLads", website: "https://www.madlads.com", marketplaces: [meSol("mad_lads")] },
  smb_gen2: { x: "SolanaMBS", website: "https://solanamonkey.business", marketplaces: [meSol("solana_monkey_business")] },
  claynosaurz: { x: "Claynosaurz", website: "https://www.claynosaurz.com", marketplaces: [meSol("claynosaurz")] },
  famous_fox_federation: { x: "FamousFoxFed", website: "https://famousfoxes.com", marketplaces: [meSol("famous_fox_federation")] },
  degods: { x: "DeGodsNFT", website: "https://de.xyz", marketplaces: [meSol("degods")] },
  galactic_geckos: { x: "GalacticGeckoSG", website: "https://galacticgeckos.app", marketplaces: [meSol("galactic_geckos")] },
  taiyo_robotics: { x: "TaiyoRobotics", website: "https://taiyorobotics.com", marketplaces: [meSol("taiyo_robotics")] },
  okay_bears: { x: "okaybears", website: "https://www.okaybears.com", marketplaces: [meSol("okay_bears")] },
  smb_gen3: { x: "SolanaMBS", xNote: "SMB's account, shared with SMB Gen2", website: "https://solanamonkey.business", marketplaces: [meSol("smb_gen3")] },
  transdimensional_fox_federation: { x: "FamousFoxFed", xNote: "Famous Fox Federation's account", website: "https://famousfoxes.com", marketplaces: [meSol("transdimensional_fox_federation")] },
  pythenians: { x: "PytheniansNFT", website: "https://pythenians.xyz", marketplaces: [meSol("pythenians")] },
  cyber_frogs: { x: "CyberFrogsNFT", website: "https://cyberfrogs.io", marketplaces: [meSol("cyber_frogs")] },
  // the Robinhood Chain collections: OpenSea at deployments/launch/robinhood-collections.json's `openseaSlug`; the launch file names no X
  // account or website, so none is shown
  stonkbrokers: { marketplaces: [os("stonkbrokers-434284142")] },
  chainmancers: { marketplaces: [os("chain-mancers")] },
  rekttradooors: { marketplaces: [os("rekt-tradooor")] },
};

export const linksOf = (slug: string): CollectionLinks | null => COLLECTION_LINKS[slug] ?? null;
export const xUrlOf = (handle: string): string => `https://x.com/${handle}`;
/** "miladymaker.net", "sovrn.art/cents": a website without its scheme, for display. */
export const siteLabel = (url: string): string => url.replace(/^https?:\/\//, "").replace(/\/$/, "");

/**
 * The links the Launch page prefills for a coin on a collection, from this verified list: the collection's X (https://x.com/<handle>,
 * when it has one) and its website (when it has one; otherwise its first marketplace page, `websiteFrom` naming the marketplace).
 * Only a starting point: the creator can change or clear them, and the coin's metadata carries what the creator sends.
 * null for a slug that is not a Rook collection.
 */
export type CoinLinks = { twitter?: string; website?: string; websiteFrom?: Marketplace["name"] };
export function coinLinksOf(slug: string): CoinLinks | null {
  const l = linksOf(slug);
  if (!l) return null;
  const out: CoinLinks = {};
  if (l.x) out.twitter = xUrlOf(l.x);
  if (l.website) out.website = l.website;
  else if (l.marketplaces[0]) { out.website = l.marketplaces[0].url; out.websiteFrom = l.marketplaces[0].name; }
  return out;
}
