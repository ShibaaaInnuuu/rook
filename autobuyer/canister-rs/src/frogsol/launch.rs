//! Default-live (operator decision 2026-10-12, audit C2 §4): every Inscriptions listing is live from install, so what the wake check
//! used to wait for is set from the launch files instead, before a collection's first floor reading: its purchase limit and minimum
//! (`nft-pilot.json`'s `maxPurchaseLamports` / `minPurchaseSol`), its Ordinals Wallet creators (none pinned: a protected sale carrying
//! a royalty is refused, review P-01's default), and the floor its first reading is bounded by (audit C2-12: `graduation-tiers.json`'s
//! `floor_sol`, the floor the launch pin was sized from). Keyed by the members root (the books' `ListingKind::Inscriptions`), so a
//! listing is matched by what it holds, never by its id or name. Baked in: the test below holds it to the launch files.

/// One Ordinals collection's launch values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LaunchDefault {
    pub slug: &'static str,
    /// The members root, hex (no 0x).
    pub root: &'static str,
    /// The floor its launch pin was sized from (`graduation-tiers.json` `floor_sol`), lamports per item.
    pub floor_lamports: u64,
    /// `set_purchase_limit`'s maximum (`maxPurchaseLamports`); `None`: the home's 10 SOL.
    pub max_purchase_lamports: Option<u128>,
    /// `set_purchase_limit`'s minimum (`minPurchaseSol`); `None`: the home's 1 SOL.
    pub min_purchase_lamports: Option<u128>,
    /// `set_ow_creators`: none of the launch files names a creator address.
    pub ow_creators: &'static [&'static str],
}

/// Audit C2-12: a listing's first floor reading (no floor accepted before it, nothing to clamp it to) is refused unless it is within
/// this of the launch floor: [floor / 2, floor × 3 / 2]. Later readings are clamped by the module's own move bound, as before.
pub const FIRST_FLOOR_BAND_BPS: u64 = 5_000;

const fn d(slug: &'static str, root: &'static str, floor_lamports: u64, max: Option<u128>, min: Option<u128>) -> LaunchDefault {
    LaunchDefault { slug, root, floor_lamports, max_purchase_lamports: max, min_purchase_lamports: min, ow_creators: &[] }
}

/// Bitcoin Frogs (`nft-pilot.json` `bitcoin`) and the further Ordinals collections (`inscriptions`), in the launch file's order.
pub const LAUNCH: [LaunchDefault; 15] = [
    d("bitcoin-frogs", "801f71bbb4c7ee37759a3b23da76bc3dd5ec132c0dd10edefe3dc86acfae418e", 3_010_000_000, None, None),
    d("nodemonkes", "e97c65d46ebf87c51e6503705a1ef2df4fbaf5168bd80eecab25d75b10126c09", 17_440_000_000, Some(40_000_000_000), None),
    d("omb", "b174d1b274c9499b6aba61858c0c6945ca0df0eabf71e269d59527c098dd7af6", 13_160_000_000, Some(25_000_000_000), None),
    d("bitcoin-puppets", "1aa1697ee4990df1fe27e5bf39d95f2d2b644191eb0cfbf127acf3e92de25d47", 7_700_000_000, Some(20_000_000_000), None),
    d("bitcoin-punks", "cf39e6081fd4cc6d82c01bec850f06c365a00b5b5c4086494c9e844afe91fda4", 2_360_000_000, Some(10_000_000_000), None),
    d("natcats", "e5a905f780d8dce46f9be8eac8337752a79a44dc8e8bf090943ee36fe0b925a3", 2_140_000_000, Some(10_000_000_000), None),
    d("runestone", "623653a2be69eccaa6666cb091fd6176ced3991df1a98a1bd2bc9e5a89ee7752", 1_040_000_000, Some(10_000_000_000), Some(250_000_000)),
    d("bitcoin-weirdos", "d2a7b78cdb8a5928f63130dc9f05e754d4ea96963c3533d7eb429c75d008d76c", 490_000_000, Some(10_000_000_000), Some(150_000_000)),
    d("cents", "a12652ab5011a1d680d7c69414232d1a271a54513173d189103a2afef5ce4849", 3_870_000_000, Some(10_000_000_000), Some(1_000_000_000)),
    d("rsic", "5c441d676fa812fb48a667eb00759806668b5c0a354fc447e164bf3525d82fee", 560_000_000, Some(10_000_000_000), Some(250_000_000)),
    d("pizza-ninjas", "230aa6d2b7b22d202022427e17d1445a7bf49c6e99da088a3102cd97cc847c55", 6_560_000_000, Some(10_000_000_000), Some(1_000_000_000)),
    d("quantum-cats", "b4c59b4a21feaf020b68b6efb75e88809f83041651026788b1f952ab03163094", 7_860_000_000, Some(25_000_000_000), Some(1_000_000_000)),
    d("wizards-of-ord", "ee49af729f84580fd399d60eeeac2bb9eda74a2aa9c3d072760c11695d04a7d9", 4_060_000_000, Some(20_000_000_000), Some(1_000_000_000)),
    d("taproot-wizards", "6ebe6bf01bd05768e6d2d2c3f6ebf9763a009e2b02513472450b16aba57a9b14", 35_350_000_000, Some(55_000_000_000), Some(1_000_000_000)),
    d("bitcoin-wizards", "53daf6fc65bea13920360cdd299077ee401229e7d18bddc5d6728a541f56fb8c", 10_150_000_000, Some(65_000_000_000), Some(1_000_000_000)),
];

/// The launch values of the listing whose members root is `root`.
pub fn of_root(root: &[u8; 32]) -> Option<&'static LaunchDefault> {
    let h = hex::encode(root);
    LAUNCH.iter().find(|d| d.root == h)
}

/// Audit C2-12: a listing's FIRST floor (no floor accepted yet), in lamports per item, against its launch floor: within
/// [`FIRST_FLOOR_BAND_BPS`] of it, or refused (a listing with no launch floor has nothing to bound a first floor by: refused too).
/// Later readings are not this rule's.
pub fn first_floor_ok(root: &[u8; 32], lamports_per_item: u128) -> Result<(), String> {
    let Some(d) = of_root(root) else {
        return Err("no launch floor names this listing's members root: its first floor cannot be bounded, so none is posted (audit C2-12)".into());
    };
    let f = d.floor_lamports as u128;
    let band = f * FIRST_FLOOR_BAND_BPS as u128 / 10_000;
    let (lo, hi) = (f - band, f + band);
    if lamports_per_item < lo || lamports_per_item > hi {
        return Err(format!("{}'s first floor of {lamports_per_item} lamports is outside [{lo}, {hi}], {} % around its launch floor of {f} \
            (graduation-tiers.json): refused, nothing posted (audit C2-12)", d.slug, FIRST_FLOOR_BAND_BPS / 100));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table is the launch files': every Ordinals collection of `nft-pilot.json` (Bitcoin Frogs and `inscriptions`) once, with its
    /// members root, its purchase maximum and minimum, and `graduation-tiers.json`'s floor.
    #[test]
    fn the_launch_defaults_are_the_launch_files() {
        let pilot: serde_json::Value = serde_json::from_str(include_str!("../../../../deployments/launch/nft-pilot.json")).unwrap();
        let tiers: serde_json::Value = serde_json::from_str(include_str!("../../../../deployments/launch/graduation-tiers.json")).unwrap();
        let rows: Vec<&serde_json::Value> = std::iter::once(&pilot["bitcoin"]).chain(pilot["inscriptions"].as_array().unwrap()).collect();
        assert_eq!(rows.len(), LAUNCH.len());
        for (r, d) in rows.iter().zip(LAUNCH.iter()) {
            assert_eq!(r["slug"].as_str().unwrap(), d.slug);
            assert_eq!(r["membersRoot"].as_str().unwrap().trim_start_matches("0x"), d.root, "{}", d.slug);
            assert_eq!(r["maxPurchaseLamports"].as_u64().map(u128::from), d.max_purchase_lamports, "{}", d.slug);
            let min = r["minPurchaseSol"].as_str().map(|s| s.parse::<f64>().unwrap()).or(r["minPurchaseSol"].as_f64());
            assert_eq!(min.map(|m| (m * 1e9).round() as u128), d.min_purchase_lamports, "{}", d.slug);
            let floor = tiers["collections"][d.slug]["floor_sol"].as_str().unwrap();
            let (w, f) = floor.split_once('.').unwrap();
            assert_eq!(w.parse::<u64>().unwrap() * 1_000_000_000 + format!("{f:0<9}").parse::<u64>().unwrap(), d.floor_lamports, "{}", d.slug);
            assert!(d.ow_creators.is_empty());
            // each set_purchase_limit would accept it
            let mut fl = crate::floortrigger::FloorState::default();
            if let Some(max) = d.max_purchase_lamports {
                fl.set_limit_min_in([1; 20], max, d.min_purchase_lamports, crate::frogsol::ABS_MIN_PURCHASE_LAMPORTS, crate::frogsol::MIN_PURCHASE_LAMPORTS,
                    crate::frogsol::RELAY_SOL_BTC_MAX_LAMPORTS, "lamports").unwrap_or_else(|e| panic!("{}: {e}", d.slug));
            }
        }
        let roots: std::collections::BTreeSet<&str> = LAUNCH.iter().map(|d| d.root).collect();
        assert_eq!(roots.len(), LAUNCH.len(), "the members roots are distinct (C2-14)");
    }

    #[test]
    fn a_first_floor_is_bounded_by_the_launch_floor() {
        let frogs: [u8; 32] = hex::decode(LAUNCH[0].root).unwrap().try_into().unwrap();
        assert!(first_floor_ok(&frogs, 3_010_000_000).is_ok());
        assert!(first_floor_ok(&frogs, 1_505_000_000).is_ok() && first_floor_ok(&frogs, 4_515_000_000).is_ok(), "the band's edges");
        assert!(first_floor_ok(&frogs, 1_504_999_999).unwrap_err().contains("outside"));
        assert!(first_floor_ok(&frogs, 4_515_000_001).unwrap_err().contains("outside"));
        assert!(first_floor_ok(&[7; 32], 3_010_000_000).unwrap_err().contains("no launch floor"));
    }
}
