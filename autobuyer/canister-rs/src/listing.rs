//! The checks that make the operator's word unnecessary (`docs/v2/trust-points.md` §1).
//!
//! Until these, a purchase's recipient, rune and spending limit were the operator's inputs, so a stolen operator key was a
//! custody key. Now every order and every quote is checked against what the chain records, read through the same path the
//! purchase policy uses (`looper::read_listing`, `policy::ret_listing`, `policy::script_of`) and bounded by the same reference
//! rate the bridge judge uses (`rates`, `oracle`) — one listing read and one rate path, not two.
//!
//! Pure: the reads live in `looper` and `canister`.

use crate::evmtx::VaultPurchase;
use crate::policy::{script_of, Listing};
use crate::psbt::Quote;

/// The listing is one the buyer may buy for at all: ours, a rune, not paused.
fn check_listing(l: &Listing, our_vault: &[u8; 20]) -> Result<(), String> {
    if &l.vault != our_vault {
        return Err(format!("refused: the listing's vault is 0x{}, not this buyer's 0x{}", hex::encode(l.vault), hex::encode(our_vault)));
    }
    if !l.is_rune {
        return Err("refused: the listing is not a rune listing; the buyer's quotes and PSBT checks are for runes only".into());
    }
    if l.paused {
        return Err("refused: the listing is paused".into());
    }
    Ok(())
}

fn listing_script(l: &Listing, hrp: &str) -> Result<Vec<u8>, String> {
    script_of(&l.btc_vault, hrp).map_err(|e| format!("refused: the listing's btcVault {:?}: {e}", l.btc_vault))
}

/// An order, before anything is announced: its rune and recipient are the listing's, not the operator's choice.
pub fn check_order(rune: &str, recipient_script: &[u8], l: &Listing, our_vault: &[u8; 20], hrp: &str) -> Result<(), String> {
    check_listing(l, our_vault)?;
    if crate::looper::parse_rune(rune)? != crate::looper::parse_rune(&l.reference)? {
        return Err(format!("refused: the order is for rune {rune:?} and the listing is for {:?}", l.reference));
    }
    let want = listing_script(l, hrp)?;
    if recipient_script != want.as_slice() {
        return Err(format!("refused: the order pays {} and the listing's address {} is {}", hex::encode(recipient_script),
            l.btc_vault, hex::encode(&want)));
    }
    Ok(())
}

/// Everything a quote is checked against, read from chain (and the canister's own key) at the moment it is checked.
pub struct QuoteFacts<'a> {
    pub listing: &'a Listing,
    pub our_vault: [u8; 20],
    /// The installed network's address prefix (`policy::hrp_for`).
    pub hrp: &'a str,
    /// The vault's own record of this purchase.
    pub purchase: &'a VaultPurchase,
    /// The canister's own taproot script.
    pub our_script: &'a [u8],
    /// `Io::reference_cap_sats(maxEth − returned)`: the most the released ETH is worth by our own rate, tolerance included.
    pub cap_sats: u64,
}

/// A quote is recordable (and, checked again, signable) only if every field that moves money is what the chain says.
pub fn check_quote(q: &Quote, f: &QuoteFacts) -> Result<(), String> {
    check_listing(f.listing, &f.our_vault)?;
    if f.purchase.status != 2 {
        return Err(format!("refused: the vault's purchase is in status {}, not Pulled: no ETH has left the vault for it", f.purchase.status));
    }
    if q.rune != crate::looper::parse_rune(&f.listing.reference)? {
        return Err(format!("refused: the quote is for rune {:?} and the listing is for {:?}", q.rune, f.listing.reference));
    }
    let want = listing_script(f.listing, f.hrp)?;
    if q.recipient_script != want {
        return Err(format!("refused: the quote pays {}, and the listing's address {} is {}", hex::encode(&q.recipient_script),
            f.listing.btc_vault, hex::encode(&want)));
    }
    if q.our_script != f.our_script {
        return Err("refused: the quote's change script is not this canister's own".into());
    }
    if q.units < f.purchase.quantity {
        return Err(format!("refused: the quote delivers {} units and the vault announced {}", q.units, f.purchase.quantity));
    }
    if q.max_spend_sats > f.cap_sats {
        return Err(format!("refused: the quote may spend {} sats; the ETH the vault released is worth at most {} by our reference \
            rate, tolerance included", q.max_spend_sats, f.cap_sats));
    }
    if q.max_fee_sats > q.max_spend_sats {
        return Err("refused: the quote's fee cap is above its spend cap".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::{hrp_for, ret_listing, LISTING, REGISTRY, LISTING_ID};
    use crate::runestone::RuneId;

    /// `cast abi-encode "f((uint8,bool,bool,uint8,address,bytes32,string,string,string))"` of a rune listing whose vault is
    /// 0x…aa and whose btcVault is the rehearsal's funded signet address.
    const ENCODED: &str = "0000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000aa00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000120000000000000000000000000000000000000000000000000000000000000016000000000000000000000000000000000000000000000000000000000000001a0000000000000000000000000000000000000000000000000000000000000001a444f47e280a2474fe280a2544fe280a2544845e280a24d4f4f4e00000000000000000000000000000000000000000000000000000000000000000000000000083834303030303a33000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000003e74623170613768367371636b786e6167736170666a34637a6733763230796c3936333039647833786a363379343230613671366a71757773356373307a370000";
    const FUNDED: &str = "tb1pa7h6sqckxnagsapfj4czg3v20yl96309dx3xj63y420a6q6jquws5cs0z7";
    /// the funded address's script: OP_1 <the canister's output key efafa803…071d>
    const FUNDED_SCRIPT: &str = "5120efafa8031634fa887429957024458a793e5d45e569a2696a24aa9fdd0352071d";
    const VAULT: [u8; 20] = { let mut v = [0u8; 20]; v[19] = 0xaa; v };

    fn listing() -> Listing {
        ret_listing(&hex::decode(ENCODED).unwrap()).unwrap()
    }
    fn purchase() -> VaultPurchase {
        VaultPurchase { status: 2, ready_at: 0, ready_at_block: 0, deadline: 0, late: false, max_eth: 10u128.pow(17), quantity: 1_000,
            settled_quantity: 0, returned: 0, cap_ref: "k".into() }
    }
    fn ours() -> Vec<u8> {
        hex::decode("5120").unwrap().into_iter().chain([7u8; 32]).collect()
    }
    fn quote() -> Quote {
        Quote { rune: RuneId { block: 840_000, tx: 3 }, units: 1_000, max_spend_sats: 100_000, max_fee_sats: 5_000,
            recipient_script: hex::decode(FUNDED_SCRIPT).unwrap(), our_script: ours() }
    }

    #[test]
    fn the_shared_reads_are_the_contracts_own_and_decode_as_foundry_encoded_them() {
        // `cast sig`: registry() 0x7b103999, listingId() 0x9405dfdd, listing(uint256) 0x84a184de
        assert_eq!((REGISTRY, LISTING_ID, LISTING), ([0x7b, 0x10, 0x39, 0x99], [0x94, 0x05, 0xdf, 0xdd], [0x84, 0xa1, 0x84, 0xde]));
        let l = listing();
        assert_eq!((l.is_rune, l.paused, l.autonomous, l.divisibility, l.vault), (true, false, true, 0, VAULT));
        assert_eq!((l.reference.as_str(), l.btc_vault.as_str()), ("840000:3", FUNDED));
    }

    #[test]
    fn addresses_decode_to_the_scripts_bitcoin_would_pay() {
        // BIP-173 and BIP-350 vectors, and this project's own funded address, through the policy's decoder
        assert_eq!(hex::encode(script_of("BC1QW508D6QEJXTDG4Y5R3ZARVARY0C5XW7KV8F3T4", "bc").unwrap()),
            "0014751e76e8199196d454941c45d1b3a323f1433bd6");
        assert_eq!(hex::encode(script_of("bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0", "bc").unwrap()),
            "512079be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798");
        assert_eq!(hex::encode(script_of("tb1qrp33g0q5c5txsp9arysrx4k6zdkfs4nce4xj0gdcccefvpysxf3q0sl5k7", "tb").unwrap()),
            "00201863143c14c5166804bd19203356da136c985678cd4d27a1b8c6329604903262");
        assert_eq!(hex::encode(script_of(FUNDED, "tb").unwrap()), FUNDED_SCRIPT, "the canister's own key");
        assert!(script_of("bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqh2y7hd", "bc").is_err(), "v1 with a bech32 checksum");
        assert!(script_of("tb1Pa7h6sqckxnagsapfj4czg3v20yl96309dx3xj63y420a6q6jquws5cs0z7", "tb").is_err(), "mixed case");
        assert!(script_of(FUNDED, "bc").is_err(), "a signet address is not a mainnet one");
        assert!(script_of(&FUNDED.replace("a7h6", "a7h7"), "tb").is_err(), "a flipped character");
        assert_eq!((hrp_for("signet").unwrap(), hrp_for("mainnet").unwrap()), ("tb", "bc"));
    }

    #[test]
    fn an_order_is_for_the_listings_rune_and_pays_the_listings_address() {
        let (l, s) = (listing(), hex::decode(FUNDED_SCRIPT).unwrap());
        assert_eq!(check_order("840000:3", &s, &l, &VAULT, "tb"), Ok(()));
        assert!(check_order("840000:3", &ours(), &l, &VAULT, "tb").unwrap_err().contains("the order pays"),
            "the operator's own script is refused: the recipient is the listing's");
        assert!(check_order("840000:4", &s, &l, &VAULT, "tb").unwrap_err().contains("listing is for"));
        assert!(check_order("840000:3", &s, &l, &[0u8; 20], "tb").unwrap_err().contains("not this buyer's"));
        assert!(check_order("840000:3", &s, &Listing { paused: true, ..l.clone() }, &VAULT, "tb").unwrap_err().contains("paused"));
        assert!(check_order("840000:3", &s, &Listing { is_rune: false, ..l.clone() }, &VAULT, "tb").unwrap_err().contains("not a rune"));
        assert!(check_order("840000:3", &s, &l, &VAULT, "bc").unwrap_err().contains("btcVault"), "a tb1 listing on a bc1 install");
    }

    #[test]
    fn a_quote_moves_money_only_where_and_as_the_chain_says() {
        let (l, v, o) = (listing(), purchase(), ours());
        let f = |cap: u64, v: &VaultPurchase| -> Result<(), String> {
            check_quote(&quote(), &QuoteFacts { listing: &l, our_vault: VAULT, hrp: "tb", purchase: v, our_script: &o, cap_sats: cap })
        };
        assert_eq!(f(100_000, &v), Ok(()));
        assert!(f(99_999, &v).unwrap_err().contains("worth at most 99999"), "a spend over the reference cap");
        assert!(f(100_000, &VaultPurchase { status: 1, ..v.clone() }).unwrap_err().contains("not Pulled"));
        assert!(f(100_000, &VaultPurchase { quantity: 1_001, ..v.clone() }).unwrap_err().contains("vault announced 1001"));
        let with = |q: Quote| check_quote(&q, &QuoteFacts { listing: &l, our_vault: VAULT, hrp: "tb", purchase: &v, our_script: &o, cap_sats: 100_000 });
        // the operator's old power, one field at a time: each is refused now
        assert!(with(Quote { recipient_script: o.clone(), ..quote() }).unwrap_err().contains("the quote pays"), "paying itself");
        assert!(with(Quote { our_script: vec![0x51, 0x20, 9], ..quote() }).unwrap_err().contains("change script"), "change elsewhere");
        assert!(with(Quote { rune: RuneId { block: 1, tx: 1 }, ..quote() }).unwrap_err().contains("listing is for"), "another rune");
        assert!(with(Quote { max_fee_sats: 100_001, ..quote() }).unwrap_err().contains("fee cap"));
        assert_eq!(with(Quote { units: 5_000, ..quote() }), Ok(()), "more units than announced is fine");
    }
}
