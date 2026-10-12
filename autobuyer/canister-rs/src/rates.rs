//! The buyer's reference rate for ETH in BTC (`docs/v2/rates-and-indexer.md` §1), and, for the Solana home, SOL in ETH
//! (docs/v9/solana-migration.md §2.3: the advance is in lamports, the NFT is priced in ETH). Pure: no network, no replica.
//!
//! The bridge step needs a floor that is never the bridge's own number. It is read from several keyless public tickers (and,
//! on mainnet, the IC's exchange rate canister), each reduced to one quantised number before consensus, then combined by a
//! median that one wrong source cannot move, and refused outright when the sources disagree or too few answer.

/// Satoshis in one bitcoin, and wei in one ether.
const SATS: u128 = 100_000_000;
const WEI: u128 = 1_000_000_000_000_000_000;

/// At least this many sources must answer, or there is no rate.
pub const MIN_SOURCES: usize = 3;
/// `(max − min) / median` above this, in basis points, and the sources disagree about the market: no rate.
pub const MAX_SPREAD_BPS: u64 = 100;
/// The floor is the rate less this, in basis points: a bridge's fee plus the quantisation step. A policy constant.
pub const TOLERANCE_BPS: u64 = 300;
/// XRC's answer is used only if its timestamp is at most this old, in seconds, by the canister's clock.
pub const XRC_MAX_AGE_SECS: u64 = 600;

/// The response cap of one ticker read (`oracle::get`), headers included. Measured 2026-10-09 (headers + body, every source and
/// market the canister reads): Coinbase 1,638 B (1,455 of them headers), OKX 1,329, crypto.com 1,166, KuCoin 990, HTX 710. A cap
/// is paid for whether it is used or not (≈ 10,400 cycles a byte per read on 13 nodes): 5,000 keeps 3× the largest answer and
/// costs ≈ 0.11 B cycles a read less than the 16,000 it replaces (≈ 1.1 B a SOL/ETH or SOL/BTC rate: ten legs).
pub const TICKER_BYTES: u64 = 5_000;
/// The largest ticker answer measured (Coinbase, headers + body, 2026-10-09).
pub const TICKER_MEASURED_MAX: u64 = 1_638;

/// The five keyless, IPv6-reachable ETH-BTC tickers. The order is the transform's context byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Coinbase,
    Okx,
    Kucoin,
    CryptoCom,
    Htx,
}

/// What a ticker is read for: ETH in BTC (the buyer's reference rate), or one leg of SOL in ETH (docs/v9 §2.3): few venues list
/// SOL-ETH, every one lists SOL and ETH against the dollar, so each source's SOL/ETH is its own ETH-USD over its own SOL-USD.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Market {
    EthBtc,
    SolUsd,
    EthUsd,
    /// Phase B2: the dollar leg of SOL in BTC (each source's SOL-USD over its BTC-USD).
    BtcUsd,
}

impl Market {
    pub const ALL: [Market; 4] = [Market::EthBtc, Market::SolUsd, Market::EthUsd, Market::BtcUsd];
    /// The transform context's second byte (absent: ETH-BTC, as every context written before the Solana home).
    pub fn from_context(b: &[u8]) -> Option<Market> {
        match b.get(1) {
            None => Some(Market::EthBtc),
            Some(&i) => Market::ALL.get(i as usize).copied(),
        }
    }
    fn index(self) -> u8 {
        Market::ALL.iter().position(|m| *m == self).unwrap_or(0) as u8
    }
    /// Decimal places kept: satoshis for ETH-BTC, micro-dollars for a dollar leg.
    fn places(self) -> u32 {
        match self {
            Market::EthBtc => 8,
            _ => 6,
        }
    }
    /// Significant figures kept before consensus: three for ETH-BTC (as always); four for a dollar leg (three would move a ~$110 SOL
    /// in whole-dollar steps, 0.9 %).
    fn figures(self) -> u32 {
        match self {
            Market::EthBtc => 3,
            _ => 4,
        }
    }
}

impl Source {
    pub const ALL: [Source; 5] = [Source::Coinbase, Source::Okx, Source::Kucoin, Source::CryptoCom, Source::Htx];

    pub fn name(self) -> &'static str {
        match self {
            Source::Coinbase => "coinbase",
            Source::Okx => "okx",
            Source::Kucoin => "kucoin",
            Source::CryptoCom => "crypto.com",
            Source::Htx => "htx",
        }
    }

    pub fn url(self) -> &'static str {
        match self {
            Source::Coinbase => "https://api.exchange.coinbase.com/products/ETH-BTC/ticker",
            Source::Okx => "https://www.okx.com/api/v5/market/ticker?instId=ETH-BTC",
            Source::Kucoin => "https://api.kucoin.com/api/v1/market/orderbook/level1?symbol=ETH-BTC",
            Source::CryptoCom => "https://api.crypto.com/exchange/v1/public/get-tickers?instrument_name=ETH_BTC",
            Source::Htx => "https://api.huobi.pro/market/detail/merged?symbol=ethbtc",
        }
    }

    /// The instrument this source names `m` by (Coinbase quotes the dollar itself; the others USDT).
    pub fn instrument(self, m: Market) -> &'static str {
        match (self, m) {
            (Source::Coinbase | Source::Okx | Source::Kucoin, Market::EthBtc) => "ETH-BTC",
            (Source::Coinbase, Market::SolUsd) => "SOL-USD",
            (Source::Coinbase, Market::EthUsd) => "ETH-USD",
            (Source::Okx | Source::Kucoin, Market::SolUsd) => "SOL-USDT",
            (Source::Okx | Source::Kucoin, Market::EthUsd) => "ETH-USDT",
            (Source::CryptoCom, Market::EthBtc) => "ETH_BTC",
            (Source::CryptoCom, Market::SolUsd) => "SOL_USDT",
            (Source::CryptoCom, Market::EthUsd) => "ETH_USDT",
            (Source::Htx, Market::EthBtc) => "ethbtc",
            (Source::Htx, Market::SolUsd) => "solusdt",
            (Source::Htx, Market::EthUsd) => "ethusdt",
            (Source::Coinbase, Market::BtcUsd) => "BTC-USD",
            (Source::Okx | Source::Kucoin, Market::BtcUsd) => "BTC-USDT",
            (Source::CryptoCom, Market::BtcUsd) => "BTC_USDT",
            (Source::Htx, Market::BtcUsd) => "btcusdt",
        }
    }

    /// The ticker's URL for `m` (ETH-BTC: [`Source::url`], unchanged).
    pub fn url_of(self, m: Market) -> String {
        let i = self.instrument(m);
        match self {
            Source::Coinbase => format!("https://api.exchange.coinbase.com/products/{i}/ticker"),
            Source::Okx => format!("https://www.okx.com/api/v5/market/ticker?instId={i}"),
            Source::Kucoin => format!("https://api.kucoin.com/api/v1/market/orderbook/level1?symbol={i}"),
            Source::CryptoCom => format!("https://api.crypto.com/exchange/v1/public/get-tickers?instrument_name={i}"),
            Source::Htx => format!("https://api.huobi.pro/market/detail/merged?symbol={i}"),
        }
    }

    /// The transform context of a read of `m`: the source byte, then the market byte.
    pub fn context_of(self, m: Market) -> Vec<u8> {
        let mut c = self.context();
        c.push(m.index());
        c
    }

    pub fn from_context(b: &[u8]) -> Option<Source> {
        b.first().and_then(|&i| Source::ALL.get(i as usize).copied())
    }

    pub fn context(self) -> Vec<u8> {
        vec![Source::ALL.iter().position(|s| *s == self).unwrap_or(0) as u8]
    }

    /// The last-trade price of `m`, as the source writes it: a decimal string, or (HTX) a JSON number.
    fn price(self, m: Market, v: &serde_json::Value) -> Option<String> {
        let s = |x: &serde_json::Value| match x {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Number(n) => Some(n.to_string()),
            _ => None,
        };
        match self {
            Source::Coinbase => s(v.get("price")?),
            Source::Okx => {
                if v.get("code")?.as_str()? != "0" { return None; }
                let d = v.get("data")?.as_array()?.first()?;
                if d.get("instId")?.as_str()? != self.instrument(m) { return None; }
                s(d.get("last")?)
            }
            Source::Kucoin => {
                if v.get("code")?.as_str()? != "200000" { return None; }
                s(v.get("data")?.get("price")?)
            }
            Source::CryptoCom => {
                if v.get("code")?.as_i64()? != 0 { return None; }
                let d = v.get("result")?.get("data")?.as_array()?.first()?;
                if d.get("i")?.as_str()? != self.instrument(m) { return None; }
                s(d.get("a")?)
            }
            Source::Htx => {
                if v.get("status")?.as_str()? != "ok" { return None; }
                s(v.get("tick")?.get("close")?)
            }
        }
    }

    /// One source's reply reduced to what consensus compares: its price in satoshis per ETH, quantised down to three
    /// significant figures, or a fixed error. Everything else in the reply — timestamps, sequence numbers, sizes — is gone.
    pub fn project(self, status: u16, body: &[u8]) -> Result<u64, String> {
        self.project_market(Market::EthBtc, status, body)
    }

    /// [`Source::project`] for any market: its price scaled to `m`'s places (satoshis, micro-dollars), quantised to its figures.
    pub fn project_market(self, m: Market, status: u16, body: &[u8]) -> Result<u64, String> {
        if status != 200 {
            return Err(format!("{}: HTTP {status}", self.name()));
        }
        let v: serde_json::Value = serde_json::from_slice(body).map_err(|_| format!("{}: not JSON", self.name()))?;
        let p = self.price(m, &v).ok_or_else(|| format!("{}: no {} price in the reply", self.name(), self.instrument(m)))?;
        let x = decimal_scaled(&p, m.places()).ok_or_else(|| format!("{}: unreadable price {p:?}", self.name()))?;
        let x = u64::try_from(x).map_err(|_| format!("{}: price out of range", self.name()))?;
        if x == 0 {
            return Err(format!("{}: a zero price", self.name()));
        }
        Ok(quantise_to(x, m.figures()))
    }
}

/// A non-negative decimal string times 10^`places`, exactly, truncating any further digits. `"0.031875"` at 8 places is
/// 3,187,500. Refuses signs, exponents and anything that is not digits and at most one point.
pub fn decimal_scaled(s: &str, places: u32) -> Option<u128> {
    let s = s.trim();
    let (int, frac) = match s.split_once('.') {
        Some((i, f)) => (i, f),
        None => (s, ""),
    };
    if int.is_empty() && frac.is_empty() { return None; }
    if !int.chars().all(|c| c.is_ascii_digit()) || !frac.chars().all(|c| c.is_ascii_digit()) { return None; }
    let mut v: u128 = if int.is_empty() { 0 } else { int.parse().ok()? };
    for i in 0..places as usize {
        let d = frac.as_bytes().get(i).map(|b| (b - b'0') as u128).unwrap_or(0);
        v = v.checked_mul(10)?.checked_add(d)?;
    }
    Some(v)
}

/// Down to three significant figures: 3,187,500 → 3,180,000. Replicas reading a moving price a second apart then produce the
/// same bytes unless the price crosses a step (~0.3%) inside that second.
pub fn quantise(x: u64) -> u64 {
    quantise_to(x, 3)
}

/// Down to `figures` significant figures.
pub fn quantise_to(x: u64, figures: u32) -> u64 {
    let digits = x.checked_ilog10().unwrap_or(0) + 1;
    if digits <= figures { return x; }
    let step = 10u64.pow(digits - figures);
    x / step * step
}

/// The IC exchange rate canister's ETH/BTC answer as satoshis per ETH, or why it is not used: too old by the canister's clock,
/// too few rates behind it, or an unusable number. `rate` is scaled by 10^`decimals`.
pub fn xrc_sats_per_eth(rate: u64, decimals: u32, timestamp_s: u64, now_s: u64, base_rates: u64, quote_rates: u64)
    -> Result<u64, String> {
    if now_s.saturating_sub(timestamp_s) > XRC_MAX_AGE_SECS {
        return Err(format!("xrc: its rate is {} s old", now_s.saturating_sub(timestamp_s)));
    }
    if timestamp_s > now_s + 60 {
        return Err("xrc: a rate dated in the future".into());
    }
    if base_rates < 2 || quote_rates < 2 {
        return Err(format!("xrc: only {base_rates}/{quote_rates} rates behind it"));
    }
    let scale = 10u128.checked_pow(decimals).ok_or("xrc: absurd decimals")?;
    let sats = (rate as u128).checked_mul(SATS).ok_or("xrc: overflow")? / scale;
    let sats = u64::try_from(sats).map_err(|_| "xrc: out of range".to_string())?;
    if sats == 0 { return Err("xrc: a zero rate".into()); }
    Ok(quantise(sats))
}

/// What the sources agreed on.
#[derive(Debug, Clone, PartialEq, Eq, candid::CandidType, serde::Deserialize, serde::Serialize)]
pub struct Rate {
    /// The median, in satoshis per ETH.
    pub sats_per_eth: u64,
    /// `(max − min) / median`, in basis points.
    pub spread_bps: u64,
    /// Each source and what it said: a price, or why not.
    pub readings: Vec<(String, Result<u64, String>)>,
}

/// Combine the readings: at least `MIN_SOURCES` must answer, their spread must be at most `MAX_SPREAD_BPS`, and the rate is
/// their (lower) median.
pub fn aggregate(readings: Vec<(String, Result<u64, String>)>) -> Result<Rate, String> {
    let mut ok: Vec<u64> = readings.iter().filter_map(|(_, r)| r.as_ref().ok().copied()).collect();
    if ok.len() < MIN_SOURCES {
        return Err(format!("only {} of {} rate sources answered; {MIN_SOURCES} are needed", ok.len(), readings.len()));
    }
    ok.sort_unstable();
    let median = ok[(ok.len() - 1) / 2];
    let spread_bps = ((ok[ok.len() - 1] - ok[0]) as u128 * 10_000 / median as u128) as u64;
    if spread_bps > MAX_SPREAD_BPS {
        return Err(format!("the rate sources disagree by {spread_bps} bps (at most {MAX_SPREAD_BPS}): {ok:?}"));
    }
    Ok(Rate { sats_per_eth: median, spread_bps, readings })
}

// ---- SOL in ETH (the Solana home: docs/v9/solana-migration.md §2.3) -------------------------------------------------------------

/// Lamports in one SOL.
pub const LAMPORTS: u128 = 1_000_000_000;

/// One source's SOL/ETH, in lamports per ETH, from its two dollar legs (micro-dollars per ETH, micro-dollars per SOL), rounded down.
pub fn lamports_per_eth(eth_usd: u64, sol_usd: u64) -> Result<u64, String> {
    if eth_usd == 0 || sol_usd == 0 {
        return Err("a zero dollar leg".into());
    }
    u64::try_from(eth_usd as u128 * LAMPORTS / sol_usd as u128).map_err(|_| "SOL/ETH out of range".to_string())
}

/// The IC exchange rate canister's SOL/ETH (base SOL, quote ETH: ETH per SOL × 10^`decimals`) as lamports per ETH, under the same
/// freshness and backing rules as [`xrc_sats_per_eth`].
pub fn xrc_lamports_per_eth(rate: u64, decimals: u32, timestamp_s: u64, now_s: u64, base_rates: u64, quote_rates: u64) -> Result<u64, String> {
    if now_s.saturating_sub(timestamp_s) > XRC_MAX_AGE_SECS {
        return Err(format!("xrc: its rate is {} s old", now_s.saturating_sub(timestamp_s)));
    }
    if timestamp_s > now_s + 60 {
        return Err("xrc: a rate dated in the future".into());
    }
    if base_rates < 2 || quote_rates < 2 {
        return Err(format!("xrc: only {base_rates}/{quote_rates} rates behind it"));
    }
    if rate == 0 {
        return Err("xrc: a zero rate".into());
    }
    let scale = 10u128.checked_pow(decimals).ok_or("xrc: absurd decimals")?;
    let l = scale.checked_mul(LAMPORTS).ok_or("xrc: overflow")? / rate as u128;
    u64::try_from(l).map_err(|_| "xrc: out of range".to_string())
}

/// What the sources agreed on for SOL in ETH.
#[derive(Debug, Clone, PartialEq, Eq, candid::CandidType, serde::Deserialize, serde::Serialize)]
pub struct SolRate {
    /// The (lower) median, in lamports per ETH.
    pub lamports_per_eth: u64,
    pub spread_bps: u64,
    pub readings: Vec<(String, Result<u64, String>)>,
}

/// [`aggregate`]'s rule for SOL in ETH: at least `MIN_SOURCES` answering, within `MAX_SPREAD_BPS` of each other, the lower median.
pub fn aggregate_sol(readings: Vec<(String, Result<u64, String>)>) -> Result<SolRate, String> {
    let r = aggregate(readings).map_err(|e| format!("SOL/ETH: {e}"))?;
    Ok(SolRate { lamports_per_eth: r.sats_per_eth, spread_bps: r.spread_bps, readings: r.readings })
}

// ---- SOL in BTC (phase B2: the frog's advance is in lamports, the frog is priced in sats) ----------------------------------------------

/// One source's SOL/BTC, in satoshis per SOL, from its two dollar legs (micro-dollars per BTC, micro-dollars per SOL), rounded down.
pub fn sats_per_sol(btc_usd: u64, sol_usd: u64) -> Result<u64, String> {
    if btc_usd == 0 || sol_usd == 0 {
        return Err("a zero dollar leg".into());
    }
    let s = u64::try_from(sol_usd as u128 * SATS / btc_usd as u128).map_err(|_| "SOL/BTC out of range".to_string())?;
    if s == 0 {
        return Err("SOL/BTC rounds to zero sats".into());
    }
    Ok(s)
}

/// The IC exchange rate canister's SOL/BTC (base SOL, quote BTC: BTC per SOL × 10^`decimals`) as satoshis per SOL, under the same
/// freshness and backing rules as [`xrc_sats_per_eth`].
pub fn xrc_sats_per_sol(rate: u64, decimals: u32, timestamp_s: u64, now_s: u64, base_rates: u64, quote_rates: u64) -> Result<u64, String> {
    if now_s.saturating_sub(timestamp_s) > XRC_MAX_AGE_SECS {
        return Err(format!("xrc: its rate is {} s old", now_s.saturating_sub(timestamp_s)));
    }
    if timestamp_s > now_s + 60 {
        return Err("xrc: a rate dated in the future".into());
    }
    if base_rates < 2 || quote_rates < 2 {
        return Err(format!("xrc: only {base_rates}/{quote_rates} rates behind it"));
    }
    let scale = 10u128.checked_pow(decimals).ok_or("xrc: absurd decimals")?;
    let sats = (rate as u128).checked_mul(SATS).ok_or("xrc: overflow")? / scale;
    let sats = u64::try_from(sats).map_err(|_| "xrc: out of range".to_string())?;
    if sats == 0 {
        return Err("xrc: a zero rate".into());
    }
    // four figures, as the dollar legs the tickers' SOL/BTC comes from (three would step a ~130,000-sat SOL by 0.8 %)
    Ok(quantise_to(sats, 4))
}

/// What the sources agreed on for SOL in BTC.
#[derive(Debug, Clone, PartialEq, Eq, candid::CandidType, serde::Deserialize, serde::Serialize)]
pub struct SolBtcRate {
    /// The (lower) median, in satoshis per SOL.
    pub sats_per_sol: u64,
    pub spread_bps: u64,
    pub readings: Vec<(String, Result<u64, String>)>,
}

/// [`aggregate`]'s rule for SOL in BTC: at least `MIN_SOURCES` answering, within `MAX_SPREAD_BPS` of each other, the lower median.
pub fn aggregate_sol_btc(readings: Vec<(String, Result<u64, String>)>) -> Result<SolBtcRate, String> {
    let r = aggregate(readings).map_err(|e| format!("SOL/BTC: {e}"))?;
    Ok(SolBtcRate { sats_per_sol: r.sats_per_eth, spread_bps: r.spread_bps, readings: r.readings })
}

/// The frog flow's reference rate on the Solana home, in its units: satoshis per 10^18 LAMPORTS (the EVM home's is satoshis per 10^18
/// wei, so `frogbuy`'s arithmetic is the same in both homes): satoshis per SOL × 10^9.
pub fn sats_per_home_unit(sats_per_sol: u64) -> Result<u64, String> {
    sats_per_sol.checked_mul(LAMPORTS as u64).ok_or_else(|| "SOL/BTC out of range".to_string())
}

/// `sats` of BTC in lamports at `sats_per_sol`, rounded down (a floor's worth, posted). 0 with no rate.
pub fn sats_to_lamports(sats: u128, sats_per_sol: u64) -> u128 {
    if sats_per_sol == 0 {
        return 0;
    }
    sats.saturating_mul(LAMPORTS) / sats_per_sol as u128
}

/// `wei` of ETH in lamports at `lamports_per_eth`, rounded down (what an amount is worth, at most).
pub fn wei_to_lamports(wei: u128, lamports_per_eth: u64) -> u128 {
    wei.saturating_mul(lamports_per_eth as u128) / WEI
}

/// The same, rounded up (what an amount costs, at least).
pub fn wei_to_lamports_ceil(wei: u128, lamports_per_eth: u64) -> u128 {
    wei.saturating_mul(lamports_per_eth as u128).div_ceil(WEI)
}

/// `lamports` in wei of ETH at `lamports_per_eth`, rounded down. 0 with no rate.
pub fn lamports_to_wei(lamports: u128, lamports_per_eth: u64) -> u128 {
    if lamports_per_eth == 0 {
        return 0;
    }
    lamports.saturating_mul(WEI) / lamports_per_eth as u128
}

/// The least BTC `send_wei` is worth at `sats_per_eth`, less `TOLERANCE_BPS`, rounded down.
pub fn floor_sats(send_wei: u128, sats_per_eth: u64) -> Result<u64, String> {
    let worth = send_wei.checked_mul(sats_per_eth as u128).ok_or("floor: overflow")? / WEI;
    let floor = worth * (10_000 - TOLERANCE_BPS as u128) / 10_000;
    u64::try_from(floor).map_err(|_| "floor: out of range".to_string())
}

/// The MOST BTC `wei` is worth at `sats_per_eth`, plus `TOLERANCE_BPS`, rounded down: the cap on what a quote may spend of the
/// ETH a purchase released (`docs/v2/trust-points.md` §1). The same rate and the same tolerance as `floor_sats`, the other way.
pub fn cap_sats(wei: u128, sats_per_eth: u64) -> Result<u64, String> {
    let worth = wei.checked_mul(sats_per_eth as u128).ok_or("cap: overflow")? / WEI;
    let cap = worth * (10_000 + TOLERANCE_BPS as u128) / 10_000;
    u64::try_from(cap).map_err(|_| "cap: out of range".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(format!("{}/tests/fixtures/rates-indexer/{name}.json", env!("CARGO_MANIFEST_DIR"))).expect("fixture")
    }

    #[test]
    fn each_real_ticker_reads_as_satoshis_per_eth_quantised() {
        // captured 2026-09-23 ~02:1x UTC, ETH ≈ 0.0319 BTC
        let got: Vec<(Source, u64)> = Source::ALL.iter().map(|s| {
            let file = match s { Source::CryptoCom => "cryptocom", _ => s.name() };
            (*s, s.project(200, &fixture(file)).unwrap_or_else(|e| panic!("{e}")))
        }).collect();
        for (s, sats) in &got {
            assert!((3_150_000..=3_220_000).contains(sats), "{}: {sats}", s.name());
            assert_eq!(*sats % 10_000, 0, "{}: quantised to three significant figures", s.name());
        }
        let r = aggregate(got.iter().map(|(s, v)| (s.name().to_string(), Ok(*v))).collect()).expect("they agree");
        assert!(r.spread_bps <= 100, "real sources were within {} bps", r.spread_bps);
    }

    #[test]
    fn a_reply_that_is_not_the_right_market_is_refused_not_misread() {
        assert!(Source::Coinbase.project(429, b"{}").unwrap_err().contains("HTTP 429"));
        assert!(Source::Okx.project(200, br#"{"code":"0","data":[{"instId":"ETH-USDT","last":"3000"}]}"#).is_err(), "another pair");
        assert!(Source::Okx.project(200, br#"{"code":"51001","data":[]}"#).is_err());
        assert!(Source::Kucoin.project(200, br#"{"code":"200000","data":null}"#).is_err());
        assert!(Source::CryptoCom.project(200, br#"{"code":0,"result":{"data":[{"i":"BTC_USD","a":"60000"}]}}"#).is_err());
        assert!(Source::Htx.project(200, br#"{"status":"error","err-msg":"invalid symbol"}"#).is_err());
        assert!(Source::Coinbase.project(200, b"<html>blocked</html>").unwrap_err().contains("not JSON"));
        assert!(Source::Coinbase.project(200, br#"{"price":"-0.03"}"#).is_err(), "a sign is not a price");
        assert!(Source::Coinbase.project(200, br#"{"price":"3e-2"}"#).is_err(), "nor an exponent");
        assert!(Source::Coinbase.project(200, br#"{"price":"0"}"#).is_err(), "nor zero");
        assert_eq!(Source::Htx.project(200, br#"{"status":"ok","tick":{"close":0.031876}}"#), Ok(3_180_000), "a JSON number too");
    }

    #[test]
    fn decimals_are_exact_and_quantising_only_rounds_down() {
        assert_eq!(decimal_scaled("0.031875", 8), Some(3_187_500));
        assert_eq!(decimal_scaled("0.0318759999", 8), Some(3_187_599), "truncated past 8 places, never rounded up");
        assert_eq!(decimal_scaled("1", 8), Some(100_000_000));
        assert_eq!(decimal_scaled(".5", 2), Some(50));
        for bad in ["", ".", "1.2.3", "0x10", " - 1", "1,5"] { assert_eq!(decimal_scaled(bad, 8), None, "{bad:?}"); }
        assert_eq!(quantise(3_187_500), 3_180_000);
        assert_eq!(quantise(3_189_999), 3_180_000);
        assert_eq!(quantise(999), 999);
        assert_eq!(quantise(10_009), 10_000);
    }

    #[test]
    fn the_median_holds_against_one_wrong_source_and_disagreement_or_silence_refuses() {
        let r = |v: &[Result<u64, String>]| aggregate(v.iter().enumerate().map(|(i, x)| (format!("s{i}"), x.clone())).collect());
        // one source wildly off does not move the median, but it does blow the spread: refused, not averaged
        assert!(r(&[Ok(3_180_000), Ok(3_190_000), Ok(3_180_000), Ok(9_000_000)]).unwrap_err().contains("disagree"));
        // a close set: the lower median
        let ok = r(&[Ok(3_190_000), Ok(3_180_000), Ok(3_180_000), Ok(3_190_000)]).unwrap();
        assert_eq!((ok.sats_per_eth, ok.spread_bps), (3_180_000, 31));
        // errors are carried, not counted
        let two = r(&[Ok(3_180_000), Err("okx: HTTP 403".into()), Ok(3_190_000), Err("x".into())]);
        assert!(two.unwrap_err().contains("only 2 of 4"));
        assert!(r(&[]).is_err());
    }

    #[test]
    fn xrc_is_used_only_when_fresh_and_backed() {
        // rate 0.0319 with 9 decimals, fresh, backed by several exchanges
        assert_eq!(xrc_sats_per_eth(31_900_000, 9, 1_000, 1_300, 6, 7), Ok(3_190_000));
        assert!(xrc_sats_per_eth(31_900_000, 9, 1_000, 1_601, 6, 7).unwrap_err().contains("601 s old"));
        assert!(xrc_sats_per_eth(31_900_000, 9, 2_000, 1_000, 6, 7).unwrap_err().contains("future"));
        assert!(xrc_sats_per_eth(31_900_000, 9, 1_000, 1_000, 1, 7).unwrap_err().contains("1/7"));
        assert!(xrc_sats_per_eth(0, 9, 1_000, 1_000, 6, 7).is_err());
    }

    #[test]
    fn the_floor_is_the_rate_less_three_percent_rounded_down() {
        // 1 ETH at 3,190,000 sat/ETH is 3,190,000 sat; less 3% is 3,094,300
        assert_eq!(floor_sats(WEI, 3_190_000), Ok(3_094_300));
        // 0.25 ETH
        assert_eq!(floor_sats(WEI / 4, 3_190_000), Ok(773_575));
        // dust rounds to zero rather than up
        assert_eq!(floor_sats(1, 3_190_000), Ok(0));
        assert!(floor_sats(u128::MAX, 3_190_000).is_err());
    }

    #[test]
    fn the_sol_legs_read_as_micro_dollars_and_combine_into_lamports_per_eth() {
        // every source names its own dollar instrument, and a reply for another instrument is refused, not misread
        assert_eq!(Source::Coinbase.project_market(Market::SolUsd, 200, br#"{"price":"109.65"}"#), Ok(109_600_000), "4 figures");
        assert_eq!(Source::Okx.project_market(Market::EthUsd, 200, br#"{"code":"0","data":[{"instId":"ETH-USDT","last":"2504.31"}]}"#),
            Ok(2_504_000_000));
        assert!(Source::Okx.project_market(Market::SolUsd, 200, br#"{"code":"0","data":[{"instId":"ETH-USDT","last":"2504.31"}]}"#).is_err());
        assert!(Source::CryptoCom.project_market(Market::SolUsd, 200, br#"{"code":0,"result":{"data":[{"i":"SOL_USDT","a":"109.7"}]}}"#).is_ok());
        assert_eq!(Source::Htx.project_market(Market::SolUsd, 200, br#"{"status":"ok","tick":{"close":109.7}}"#), Ok(109_700_000));
        assert_eq!(Source::Kucoin.url_of(Market::SolUsd), "https://api.kucoin.com/api/v1/market/orderbook/level1?symbol=SOL-USDT");
        assert_eq!(Source::Coinbase.url_of(Market::EthBtc), Source::Coinbase.url(), "ETH-BTC is the same read as ever");
        assert_eq!((Market::from_context(&[2]), Market::from_context(&[2, 1])), (Some(Market::EthBtc), Some(Market::SolUsd)));
        // 1 SOL = 0.043783 ETH on 2026-10-09 (Relay's sol-eth-1): ETH $2,504.3, SOL $109.65 → 22.84 SOL an ETH
        let l = lamports_per_eth(2_504_000_000, 109_600_000).unwrap();
        assert_eq!(l, 22_846_715_328);
        assert_eq!(wei_to_lamports(WEI, l), l as u128);
        assert_eq!(lamports_to_wei(l as u128, l), WEI);
        assert_eq!(wei_to_lamports_ceil(1, l), 1, "a cost rounds up");
        assert_eq!(wei_to_lamports(1, l), 0, "a worth rounds down");
        assert!(lamports_per_eth(0, 1).is_err());
        // the sources disagreeing by more than the bound is no rate, as for ETH/BTC
        let r = |v: &[u64]| aggregate_sol(v.iter().enumerate().map(|(i, x)| (format!("s{i}"), Ok(*x))).collect());
        assert_eq!(r(&[22_846_000_000, 22_850_000_000, 22_840_000_000]).unwrap().lamports_per_eth, 22_846_000_000);
        assert!(r(&[22_846_000_000, 22_850_000_000, 24_000_000_000]).unwrap_err().contains("disagree"));
        // XRC: 0.043783 ETH per SOL at 9 decimals
        assert_eq!(xrc_lamports_per_eth(43_783_000, 9, 1_000, 1_100, 5, 5), Ok(22_839_915_035));
        assert!(xrc_lamports_per_eth(43_783_000, 9, 1_000, 1_700, 5, 5).unwrap_err().contains("old"));
        assert!(xrc_lamports_per_eth(0, 9, 1_000, 1_000, 5, 5).is_err());
    }

    #[test]
    fn the_sol_btc_legs_combine_into_sats_per_sol_and_the_frog_rate_is_per_ten_to_the_eighteen_lamports() {
        assert_eq!(Source::Coinbase.url_of(Market::BtcUsd), "https://api.exchange.coinbase.com/products/BTC-USD/ticker");
        assert_eq!(Source::Htx.instrument(Market::BtcUsd), "btcusdt");
        assert_eq!(Market::from_context(&[0, 3]), Some(Market::BtcUsd));
        assert_eq!(Source::Okx.project_market(Market::BtcUsd, 200, br#"{"code":"0","data":[{"instId":"BTC-USDT","last":"83781.2"}]}"#),
            Ok(83_780_000_000), "4 figures");
        // 2026-10-09 (spike Q5 sol-btc-1): 1 SOL bought 0.00130881 BTC after −1.53 %: ≈ 132,900 sats a SOL; SOL $109.65, BTC $82,500
        let s = sats_per_sol(82_500_000_000, 109_650_000).unwrap();
        assert_eq!(s, 132_909);
        assert!(sats_per_sol(0, 1).is_err() && sats_per_sol(1, 0).is_err());
        assert_eq!(sats_per_home_unit(s), Ok(132_909_000_000_000));
        // frogbuy's arithmetic in lamports: 900,000 sats cost 6.77 SOL at this rate, 6.91 with its 2 % margin
        let l = crate::frogbuy::wei_for_sats(900_000, sats_per_home_unit(s).unwrap()).unwrap();
        assert!((6_906_000_000..6_908_000_000).contains(&l), "{l}");
        assert_eq!(sats_to_lamports(132_909, s), LAMPORTS);
        let r = |v: &[u64]| aggregate_sol_btc(v.iter().enumerate().map(|(i, x)| (format!("s{i}"), Ok(*x))).collect());
        assert_eq!(r(&[132_909, 132_950, 132_800]).unwrap().sats_per_sol, 132_909);
        assert!(r(&[132_909, 132_950, 150_000]).unwrap_err().contains("disagree"));
        // XRC SOL/BTC: 0.00132909 BTC a SOL at 9 decimals
        assert_eq!(xrc_sats_per_sol(1_329_090, 9, 1_000, 1_100, 5, 5), Ok(132_900));
        assert!(xrc_sats_per_sol(1_329_090, 9, 1_000, 1_700, 5, 5).unwrap_err().contains("old"));
    }

    #[test]
    fn the_quote_cap_is_the_same_rate_plus_three_percent_the_floors_mirror() {
        assert_eq!(cap_sats(WEI, 3_190_000), Ok(3_285_700));
        assert_eq!(cap_sats(WEI / 4, 3_190_000), Ok(821_425));
        // the floor and the cap bracket the same reference value, 3% either side
        let (f, c) = (floor_sats(WEI, 3_190_000).unwrap(), cap_sats(WEI, 3_190_000).unwrap());
        assert!(f < 3_190_000 && 3_190_000 < c && c - 3_190_000 == 3_190_000 - f);
        assert!(cap_sats(u128::MAX, 3_190_000).is_err());
    }

    #[test]
    fn the_ticker_cap_keeps_three_times_the_largest_answer_measured() {
        assert!(TICKER_BYTES >= 3 * TICKER_MEASURED_MAX, "{TICKER_BYTES}");
        assert!(TICKER_BYTES < 16_000, "the cap is not the old 16 kB");
    }
}
