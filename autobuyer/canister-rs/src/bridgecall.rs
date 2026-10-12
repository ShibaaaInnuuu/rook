//! The bridge calls the canister would sign, decoded and judged BEFORE signing (`docs/v2/venue-bridge-policy.md` §2).
//!
//! A bridge quote's own `recipient` field is the quoter's word. What moves the money is the calldata. So a call is accepted only
//! if the calldata itself names the buyer's own destination — which rules out any bridge that binds the destination off chain.
//!
//! - **Across `depositV3`** (hop 1, 4663 ETH → Arbitrum ETH): the recipient, destination chain, tokens and minimum out are all
//!   arguments. Verified.
//! - **Chainflip `xSwapNative`** (hop 2, Arbitrum ETH → native BTC): the destination chain, asset and Bitcoin address (UTF-8 of
//!   the address string) are arguments. Verified; `cfParameters` (refund and broker data, SCALE-encoded) is checked only for
//!   naming our own refund address.
//! - **Relay `depositNative(address depositor, bytes32 id)`**: the destination lives in Relay's database under `id`. Recognised,
//!   and refused.
//!
//! Pure and tested against Foundry's `cast` and against a real Across quote (2026-09-23). Not wired into the loop: executing hop
//! 2 means operating on a second EVM chain, which the loop does not do yet, and today's Across liquidity on 4663 carries
//! only dust.

use crate::abi::{addr_at, addr_word, bytes_at, encode_call, selector_of, uint_at, uint_word, Arg};

pub const ACROSS_DEPOSIT_V3: [u8; 4] = [0x7b, 0x93, 0x92, 0x32];
pub const CHAINFLIP_X_SWAP_NATIVE: [u8; 4] = [0xdd, 0x68, 0x73, 0x45];
pub const RELAY_DEPOSIT_NATIVE: [u8; 4] = [0x49, 0x29, 0x0c, 0x1c];

/// Across's SpokePool on Robinhood Chain 4663 (`/api/suggested-fees`, 2026-09-23) and its wrapped native token (read from the
/// pool's `wrappedNativeToken()`).
pub const ACROSS_SPOKE_4663: [u8; 20] = hex20("d29c85f15df544ba632c9e25829fd29d767d7978");
pub const WETH_4663: [u8; 20] = hex20("0bd7d308f8e1639fab988df18a8011f41eacad73");
/// WETH on Arbitrum One: what an ETH deposit arrives as.
pub const WETH_ARBITRUM: [u8; 20] = hex20("82af49447d8a07e3bd95bd0d56f35241523fbab1");
pub const ARBITRUM_ONE: u64 = 42161;
/// Chainflip's Vault on Arbitrum One (docs.chainflip.io mainnet addresses, 2026-09-23); Bitcoin is chain 3, BTC asset 5
/// (chainflip-sdk `enums.ts`).
pub const CHAINFLIP_VAULT_ARBITRUM: [u8; 20] = hex20("79001a5e762f3befc8e5871b42f6734e00498920");
pub const CHAINFLIP_CHAIN_BITCOIN: u32 = 3;
pub const CHAINFLIP_ASSET_BTC: u32 = 5;

const fn hex20(s: &str) -> [u8; 20] {
    let b = s.as_bytes();
    let mut out = [0u8; 20];
    let mut i = 0;
    while i < 20 {
        out[i] = (nib(b[2 * i]) << 4) | nib(b[2 * i + 1]);
        i += 1;
    }
    out
}
const fn nib(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        _ => panic!("not lower-case hex"),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcrossDeposit {
    pub depositor: [u8; 20],
    pub recipient: [u8; 20],
    pub input_token: [u8; 20],
    pub output_token: [u8; 20],
    pub input_amount: u128,
    pub output_amount: u128,
    pub destination_chain_id: u64,
    pub exclusive_relayer: [u8; 20],
    pub quote_timestamp: u32,
    pub fill_deadline: u32,
    pub exclusivity_deadline: u32,
    pub message: Vec<u8>,
}

pub fn encode_across(d: &AcrossDeposit) -> Vec<u8> {
    encode_call(ACROSS_DEPOSIT_V3, &[
        Arg::Word(addr_word(&d.depositor)),
        Arg::Word(addr_word(&d.recipient)),
        Arg::Word(addr_word(&d.input_token)),
        Arg::Word(addr_word(&d.output_token)),
        Arg::Word(uint_word(d.input_amount)),
        Arg::Word(uint_word(d.output_amount)),
        Arg::Word(uint_word(d.destination_chain_id as u128)),
        Arg::Word(addr_word(&d.exclusive_relayer)),
        Arg::Word(uint_word(d.quote_timestamp as u128)),
        Arg::Word(uint_word(d.fill_deadline as u128)),
        Arg::Word(uint_word(d.exclusivity_deadline as u128)),
        Arg::Bytes(&d.message),
    ])
}

pub fn decode_across(data: &[u8]) -> Result<AcrossDeposit, String> {
    if selector_of(data) != Some(ACROSS_DEPOSIT_V3) {
        return Err("not an Across depositV3".into());
    }
    let b = &data[4..];
    Ok(AcrossDeposit {
        depositor: addr_at(b, 0, 0)?,
        recipient: addr_at(b, 0, 1)?,
        input_token: addr_at(b, 0, 2)?,
        output_token: addr_at(b, 0, 3)?,
        input_amount: uint_at(b, 0, 4, 128)?,
        output_amount: uint_at(b, 0, 5, 128)?,
        destination_chain_id: uint_at(b, 0, 6, 64)? as u64,
        exclusive_relayer: addr_at(b, 0, 7)?,
        quote_timestamp: uint_at(b, 0, 8, 32)? as u32,
        fill_deadline: uint_at(b, 0, 9, 32)? as u32,
        exclusivity_deadline: uint_at(b, 0, 10, 32)? as u32,
        message: bytes_at(b, 0, 11)?,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainflipSwap {
    pub dst_chain: u32,
    pub dst_address: Vec<u8>,
    pub dst_token: u32,
    pub cf_parameters: Vec<u8>,
}

pub fn decode_chainflip(data: &[u8]) -> Result<ChainflipSwap, String> {
    if selector_of(data) != Some(CHAINFLIP_X_SWAP_NATIVE) {
        return Err("not a Chainflip xSwapNative".into());
    }
    let b = &data[4..];
    Ok(ChainflipSwap {
        dst_chain: uint_at(b, 0, 0, 32)? as u32,
        dst_address: bytes_at(b, 0, 1)?,
        dst_token: uint_at(b, 0, 2, 32)? as u32,
        cf_parameters: bytes_at(b, 0, 3)?,
    })
}

pub fn encode_chainflip(s: &ChainflipSwap) -> Vec<u8> {
    encode_call(CHAINFLIP_X_SWAP_NATIVE, &[
        Arg::Word(uint_word(s.dst_chain as u128)),
        Arg::Bytes(&s.dst_address),
        Arg::Word(uint_word(s.dst_token as u128)),
        Arg::Bytes(&s.cf_parameters),
    ])
}

/// Why a bridge call was refused before signing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallRefusal {
    /// The contract is not one of the two this module knows how to judge.
    UnknownContract([u8; 20]),
    /// A call that binds its destination off chain (Relay's `depositNative`): nothing in it can be checked.
    DestinationNotInCalldata,
    /// The calldata does not decode as the function the contract implies.
    Unreadable(String),
    /// A field that must be ours, or a fixed value, is something else. The field's name.
    Wrong(&'static str),
    /// It would pay out less than our own floor allows.
    TooLittleOut { out: u128, floor: u128 },
    /// The fill deadline is already past (or so close it cannot be met).
    Expired,
}

/// What the buyer expects of a bridge call: its own EVM address (hop 1's recipient and depositor, hop 2's refund), its own
/// Bitcoin address (hop 2's destination, as the address string), the wei it is sending, and the least it will accept out.
pub struct Expect<'a> {
    pub our_evm: [u8; 20],
    pub our_btc_address: &'a str,
    pub send_wei: u128,
    pub min_out: u128,
    pub now: u64,
}

/// Judge a call to `to` with `data` and `value` before it is signed.
pub fn verify(to: [u8; 20], data: &[u8], value: u128, e: &Expect) -> Result<(), CallRefusal> {
    if selector_of(data) == Some(RELAY_DEPOSIT_NATIVE) {
        return Err(CallRefusal::DestinationNotInCalldata);
    }
    match to {
        ACROSS_SPOKE_4663 => {
            let d = decode_across(data).map_err(CallRefusal::Unreadable)?;
            if d.depositor != e.our_evm { return Err(CallRefusal::Wrong("depositor")); }
            if d.recipient != e.our_evm { return Err(CallRefusal::Wrong("recipient")); }
            if d.destination_chain_id != ARBITRUM_ONE { return Err(CallRefusal::Wrong("destinationChainId")); }
            if d.input_token != WETH_4663 { return Err(CallRefusal::Wrong("inputToken")); }
            if d.output_token != WETH_ARBITRUM { return Err(CallRefusal::Wrong("outputToken")); }
            if d.input_amount != e.send_wei || value != e.send_wei { return Err(CallRefusal::Wrong("inputAmount / value")); }
            // a message makes the fill call the recipient with data; ours is an address with no code, and we never want one
            if !d.message.is_empty() { return Err(CallRefusal::Wrong("message")); }
            if d.output_amount < e.min_out { return Err(CallRefusal::TooLittleOut { out: d.output_amount, floor: e.min_out }); }
            if (d.fill_deadline as u64) <= e.now + 60 { return Err(CallRefusal::Expired); }
            Ok(())
        }
        CHAINFLIP_VAULT_ARBITRUM => {
            let s = decode_chainflip(data).map_err(CallRefusal::Unreadable)?;
            if s.dst_chain != CHAINFLIP_CHAIN_BITCOIN { return Err(CallRefusal::Wrong("dstChain")); }
            if s.dst_token != CHAINFLIP_ASSET_BTC { return Err(CallRefusal::Wrong("dstToken")); }
            if s.dst_address != e.our_btc_address.as_bytes() { return Err(CallRefusal::Wrong("dstAddress")); }
            if value != e.send_wei { return Err(CallRefusal::Wrong("value")); }
            // cfParameters carries the refund address among other SCALE-encoded fields. A full decode is not attempted; a
            // non-empty one must at least name our own address as the place a failed swap refunds to.
            if !s.cf_parameters.is_empty() && !s.cf_parameters.windows(20).any(|w| w == e.our_evm) {
                return Err(CallRefusal::Wrong("cfParameters refund address"));
            }
            Ok(())
        }
        other => Err(CallRefusal::UnknownContract(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OURS: [u8; 20] = hex20("5b0b0649f0306d08b4c0ad37406a7082fc25ebc2");
    const OUR_BTC: &str = "bc1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqkedrcr";

    /// `cast calldata "depositV3(address,…,bytes)"` over the real Across quote of 2026-09-23 (0.01 ETH 4663 → Arbitrum):
    /// outputAmount 9992746984183277, exclusiveRelayer 0xFD03…b7F0, quoteTimestamp 1790131451, fillDeadline 1790138651,
    /// exclusivityDeadline 3.
    const ACROSS_CAST: &str = "7b9392320000000000000000000000005b0b0649f0306d08b4c0ad37406a7082fc25ebc20000000000000000000000005b0b0649f0306d08b4c0ad37406a7082fc25ebc20000000000000000000000000bd7d308f8e1639fab988df18a8011f41eacad7300000000000000000000000082af49447d8a07e3bd95bd0d56f35241523fbab1000000000000000000000000000000000000000000000000002386f26fc1000000000000000000000000000000000000000000000000000000238059b65295ed000000000000000000000000000000000000000000000000000000000000a4b1000000000000000000000000fd03abcadaf3f930fa4e37eb2f6ea3a44a41b7f0000000000000000000000000000000000000000000000000000000006ab33cfb000000000000000000000000000000000000000000000000000000006ab3591b000000000000000000000000000000000000000000000000000000000000000300000000000000000000000000000000000000000000000000000000000001800000000000000000000000000000000000000000000000000000000000000000";
    /// `cast calldata "xSwapNative(uint32,bytes,uint32,bytes)" 3 <utf8 of OUR_BTC> 5 0x`
    const CHAINFLIP_CAST: &str = "dd68734500000000000000000000000000000000000000000000000000000000000000030000000000000000000000000000000000000000000000000000000000000080000000000000000000000000000000000000000000000000000000000000000500000000000000000000000000000000000000000000000000000000000000e0000000000000000000000000000000000000000000000000000000000000003e62633170356379786e75786d65757775766b7766656d39366c717a737a6430326e367864636a727332306361633679716a6a7775647078716b656472637200000000000000000000000000000000000000000000000000000000000000000000";
    /// The deposit transaction's data from a real Relay quote (0.1 ETH 4663 → BTC, 2026-09-23), to 0x4cd00e38…bc31.
    const RELAY_REAL: &str = "49290c1c0000000000000000000000005b0b0649f0306d08b4c0ad37406a7082fc25ebc2a6e892a9d77d4c236bfdd5e1cabd0bae64883fbe6ac354653da65f1072c47475";

    fn across() -> AcrossDeposit {
        AcrossDeposit {
            depositor: OURS, recipient: OURS, input_token: WETH_4663, output_token: WETH_ARBITRUM,
            input_amount: 10_000_000_000_000_000, output_amount: 9_992_746_984_183_277, destination_chain_id: ARBITRUM_ONE,
            exclusive_relayer: hex20("fd03abcadaf3f930fa4e37eb2f6ea3a44a41b7f0"), quote_timestamp: 1_790_131_451,
            fill_deadline: 1_790_138_651, exclusivity_deadline: 3, message: vec![],
        }
    }
    fn expect() -> Expect<'static> {
        Expect { our_evm: OURS, our_btc_address: OUR_BTC, send_wei: 10_000_000_000_000_000, min_out: 9_900_000_000_000_000,
            now: 1_790_131_500 }
    }

    #[test]
    fn across_encodes_exactly_as_cast_does_and_decodes_back() {
        assert_eq!(hex::encode(encode_across(&across())), ACROSS_CAST);
        assert_eq!(decode_across(&hex::decode(ACROSS_CAST).unwrap()).unwrap(), across());
    }

    #[test]
    fn an_across_deposit_is_accepted_only_to_ourselves_on_arbitrum_for_the_amount_and_above_the_floor() {
        let e = expect();
        let ok = encode_across(&across());
        assert_eq!(verify(ACROSS_SPOKE_4663, &ok, e.send_wei, &e), Ok(()));
        let bad = |f: fn(&mut AcrossDeposit)| { let mut d = across(); f(&mut d); encode_across(&d) };
        let thief = hex20("000000000000000000000000000000000000beef");
        assert_eq!(verify(ACROSS_SPOKE_4663, &bad(|d| d.recipient = hex20("000000000000000000000000000000000000beef")), e.send_wei, &e),
            Err(CallRefusal::Wrong("recipient")), "the deposit that pays somebody else: {thief:?}");
        assert_eq!(verify(ACROSS_SPOKE_4663, &bad(|d| d.destination_chain_id = 8453), e.send_wei, &e), Err(CallRefusal::Wrong("destinationChainId")));
        assert_eq!(verify(ACROSS_SPOKE_4663, &bad(|d| d.output_token = [9; 20]), e.send_wei, &e), Err(CallRefusal::Wrong("outputToken")));
        assert_eq!(verify(ACROSS_SPOKE_4663, &bad(|d| d.message = vec![1]), e.send_wei, &e), Err(CallRefusal::Wrong("message")));
        assert_eq!(verify(ACROSS_SPOKE_4663, &bad(|d| d.output_amount = 1), e.send_wei, &e),
            Err(CallRefusal::TooLittleOut { out: 1, floor: e.min_out }));
        assert_eq!(verify(ACROSS_SPOKE_4663, &bad(|d| d.fill_deadline = 1_790_131_520), e.send_wei, &e), Err(CallRefusal::Expired));
        assert_eq!(verify(ACROSS_SPOKE_4663, &ok, e.send_wei + 1, &e), Err(CallRefusal::Wrong("inputAmount / value")),
            "a value that is not the deposit's amount");
        assert_eq!(verify([7; 20], &ok, e.send_wei, &e), Err(CallRefusal::UnknownContract([7; 20])));
    }

    #[test]
    fn chainflip_decodes_what_cast_encodes_and_accepts_only_our_bitcoin_address() {
        let s = decode_chainflip(&hex::decode(CHAINFLIP_CAST).unwrap()).unwrap();
        assert_eq!((s.dst_chain, s.dst_token, s.dst_address.as_slice(), s.cf_parameters.len()),
            (CHAINFLIP_CHAIN_BITCOIN, CHAINFLIP_ASSET_BTC, OUR_BTC.as_bytes(), 0));
        assert_eq!(hex::encode(encode_chainflip(&s)), CHAINFLIP_CAST);
        let e = expect();
        assert_eq!(verify(CHAINFLIP_VAULT_ARBITRUM, &hex::decode(CHAINFLIP_CAST).unwrap(), e.send_wei, &e), Ok(()));
        let other = ChainflipSwap { dst_address: b"bc1qsomebodyelse0000000000000000000000000".to_vec(), ..s.clone() };
        assert_eq!(verify(CHAINFLIP_VAULT_ARBITRUM, &encode_chainflip(&other), e.send_wei, &e), Err(CallRefusal::Wrong("dstAddress")));
        let eth_out = ChainflipSwap { dst_chain: 1, ..s.clone() };
        assert_eq!(verify(CHAINFLIP_VAULT_ARBITRUM, &encode_chainflip(&eth_out), e.send_wei, &e), Err(CallRefusal::Wrong("dstChain")));
        // broker parameters that do not refund to us are refused; ones that do pass
        let foreign = ChainflipSwap { cf_parameters: vec![0x42; 60], ..s.clone() };
        assert_eq!(verify(CHAINFLIP_VAULT_ARBITRUM, &encode_chainflip(&foreign), e.send_wei, &e), Err(CallRefusal::Wrong("cfParameters refund address")));
        let refunds_us = ChainflipSwap { cf_parameters: [vec![0, 1], OURS.to_vec(), vec![9; 8]].concat(), ..s };
        assert_eq!(verify(CHAINFLIP_VAULT_ARBITRUM, &encode_chainflip(&refunds_us), e.send_wei, &e), Ok(()));
    }

    #[test]
    fn relays_real_deposit_is_refused_because_its_destination_is_not_in_it() {
        let data = hex::decode(RELAY_REAL).unwrap();
        let e = expect();
        assert_eq!(verify(hex20("4cd00e387622c35bddb9b4c962c136462338bc31"), &data, 100_000_000_000_000_000, &e),
            Err(CallRefusal::DestinationNotInCalldata));
        // and it names us only as the depositor: nothing in it says where the BTC goes
        assert_eq!(&data[16..36], &OURS);
    }

    #[test]
    fn malformed_calldata_is_unreadable_not_a_pass() {
        let e = expect();
        let short = hex::decode(&ACROSS_CAST[..200]).unwrap();
        assert!(matches!(verify(ACROSS_SPOKE_4663, &short, e.send_wei, &e), Err(CallRefusal::Unreadable(_))));
        let mut wide = hex::decode(ACROSS_CAST).unwrap();
        wide[4] = 1; // a depositor word with high bytes set is not an address
        assert!(matches!(verify(ACROSS_SPOKE_4663, &wide, e.send_wei, &e), Err(CallRefusal::Unreadable(_))));
    }
}
