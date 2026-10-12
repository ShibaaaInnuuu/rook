//! The BIP341 signature hash, computed **here**, from the PSBT this canister parsed and verified.
//!
//! This module exists to close `docs/security/audit-btc-v1.md` B3. `psbt.rs` is 716 lines whose module comment is the whole
//! safety argument for the venue integration — we sign only our own inputs, what leaves us is at most the cap recorded
//! before the PSBT was asked for, the fee is bounded, the runestone delivers the asset to the recipient's script — and none
//! of it was on the path that signed. The sighash was computed in JavaScript, on the operator's machine, and handed to the
//! canister as thirty-two opaque bytes. The canister signed a number. A replicated program that cannot be talked into
//! signing is the entire point of putting the key in a subnet, and it was checking a transaction it never saw.
//!
//! The loop closes because **BIP341 commits to the input amounts and scripts**. `sha_amounts` and `sha_scriptpubkeys` are
//! part of the message, so a sighash computed from the same PSBT that `verify` read cannot describe a different transaction
//! from the one that was checked. A venue that lies about what our own input is worth produces a signature that is simply
//! invalid for the real UTXO — but only if we compute the digest from the bytes we verified rather than from bytes somebody
//! hands us afterwards. That "rather than" is the fix.
//!
//! Scope, stated rather than implied: **key-path spends with `SIGHASH_DEFAULT` and no annex**, which is what a taproot
//! output of ours is spent with and all the buyer ever needs. Anything else is refused by name rather than mis-hashed.

use crate::psbt::Psbt;
use sha2::{Digest, Sha256};

#[derive(Debug, PartialEq, Eq)]
pub enum SighashError {
    /// An input's value and script are not in the PSBT, so the message cannot commit to them.
    UnknownInputValue(usize),
    /// There is no such input in this transaction.
    NoSuchInput(usize),
    /// A sighash type this module does not compute (only `0x00` and `0x83`).
    UnsupportedHashType(u8),
    /// `SIGHASH_SINGLE` for an input with no output at its index: BIP341 refuses it.
    NoOutputAtIndex(usize),
}

/// `SHA256(SHA256(tag) || SHA256(tag) || m)`, BIP340's tagged hash.
pub(crate) fn tagged(tag: &str, parts: &[&[u8]]) -> [u8; 32] {
    let t = Sha256::digest(tag.as_bytes());
    let mut h = Sha256::new();
    h.update(t);
    h.update(t);
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}

/// Bitcoin's compact size, which prefixes every script in the message.
fn compact(n: usize) -> Vec<u8> {
    if n < 0xfd {
        vec![n as u8]
    } else if n <= 0xffff {
        let mut v = vec![0xfd];
        v.extend_from_slice(&(n as u16).to_le_bytes());
        v
    } else if n <= 0xffff_ffff {
        let mut v = vec![0xfe];
        v.extend_from_slice(&(n as u32).to_le_bytes());
        v
    } else {
        let mut v = vec![0xff];
        v.extend_from_slice(&(n as u64).to_le_bytes());
        v
    }
}

fn sha(parts: &[Vec<u8>]) -> [u8; 32] {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}

/// `SIGHASH_SINGLE | SIGHASH_ANYONECANPAY`: what a seller signs a listing with (docs/v6/canister-floor-followups.md §3.3). The
/// signature then commits to this one input (outpoint, value, script, sequence) and to the output at the same index, and to
/// nothing else a buyer adds around them.
pub const SIGHASH_SINGLE_ANYONECANPAY: u8 = 0x83;

/// The digest to sign for `input_index`, a key-path spend of a taproot output under `SIGHASH_DEFAULT`.
///
/// Every input's value and script must be known, not only the one being signed: BIP341 commits to **all** of them, which is
/// exactly the property that makes a verified PSBT and a signed digest describe the same transaction.
pub fn taproot_key_path(psbt: &Psbt, input_index: usize) -> Result<[u8; 32], SighashError> {
    taproot_key_path_typed(psbt, input_index, 0)
}

/// [`taproot_key_path`] under a stated `hash_type`: `0x00` (`SIGHASH_DEFAULT`, the buyer's and custody's spends) or `0x83`
/// (`SIGHASH_SINGLE | SIGHASH_ANYONECANPAY`, a listing). Any other type is refused by name rather than mis-hashed. Under
/// `0x83` BIP341 replaces the four all-input hashes with this input's own fields and `sha_outputs` with the hash of the one
/// output at this input's index, which must exist.
pub fn taproot_key_path_typed(psbt: &Psbt, input_index: usize, hash_type: u8) -> Result<[u8; 32], SighashError> {
    if hash_type != 0 && hash_type != SIGHASH_SINGLE_ANYONECANPAY {
        return Err(SighashError::UnsupportedHashType(hash_type));
    }
    let tx = &psbt.tx;
    if input_index >= tx.inputs.len() {
        return Err(SighashError::NoSuchInput(input_index));
    }
    if hash_type == SIGHASH_SINGLE_ANYONECANPAY && input_index >= tx.outputs.len() {
        return Err(SighashError::NoOutputAtIndex(input_index));
    }
    let mut utxos = Vec::with_capacity(tx.inputs.len());
    for i in 0..tx.inputs.len() {
        match psbt.witness_utxos.get(i).and_then(|u| u.as_ref()) {
            Some(u) => utxos.push(u),
            None => return Err(SighashError::UnknownInputValue(i)),
        }
    }

    let mut prevouts = Vec::new();
    let mut amounts = Vec::new();
    let mut scripts = Vec::new();
    let mut sequences = Vec::new();
    for (i, input) in tx.inputs.iter().enumerate() {
        let mut o = input.txid.to_vec();
        o.extend_from_slice(&input.vout.to_le_bytes());
        prevouts.push(o);
        amounts.push(utxos[i].value.to_le_bytes().to_vec());
        let mut s = compact(utxos[i].script.len());
        s.extend_from_slice(&utxos[i].script);
        scripts.push(s);
        sequences.push(input.sequence.to_le_bytes().to_vec());
    }
    let mut outputs = Vec::new();
    for o in &tx.outputs {
        let mut b = o.value.to_le_bytes().to_vec();
        b.extend_from_slice(&compact(o.script.len()));
        b.extend_from_slice(&o.script);
        outputs.push(b);
    }

    let sha_prevouts = sha(&prevouts);
    let sha_amounts = sha(&amounts);
    let sha_scriptpubkeys = sha(&scripts);
    let sha_sequences = sha(&sequences);
    let sha_outputs = sha(&outputs);

    // BIP341 SigMsg, in its order. The leading zero byte is the sighash epoch; `spend_type` is zero because this is a
    // key-path spend with no annex, and both of those are facts about what we sign rather than about the transaction.
    let epoch = [0u8];
    let hash_type_b = [hash_type];
    let spend_type = [0u8];
    let version = tx.version.to_le_bytes();
    let locktime = tx.locktime.to_le_bytes();
    let index = (input_index as u32).to_le_bytes();

    if hash_type == 0 {
        return Ok(tagged(
            "TapSighash",
            &[
                &epoch,
                &hash_type_b,
                &version,
                &locktime,
                &sha_prevouts,
                &sha_amounts,
                &sha_scriptpubkeys,
                &sha_sequences,
                &sha_outputs,
                &spend_type,
                &index,
            ],
        ));
    }
    // SIGHASH_SINGLE | SIGHASH_ANYONECANPAY: no all-input hashes and no sha_outputs; instead this input's outpoint, amount,
    // scriptPubKey and sequence, then the hash of the one output at this index.
    let sha_single_output = sha(&[outputs[input_index].clone()]);
    Ok(tagged(
        "TapSighash",
        &[
            &epoch,
            &hash_type_b,
            &version,
            &locktime,
            &spend_type,
            &prevouts[input_index],
            &amounts[input_index],
            &scripts[input_index],
            &sequences[input_index],
            &sha_single_output,
        ],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::psbt::Psbt;

    /// The digests below were produced by **@scure/btc-signer**, not by this file.
    ///
    /// `autobuyer/gen-sighash-fixture.mjs` builds each transaction with that library, asks *it* for
    /// `preimageWitnessV1(n, scripts, SigHash.DEFAULT, amounts)`, and prints the PSBT beside the answer. That library is the
    /// one whose signatures signet has already accepted from this project — the confirmed spend in block 323256 and the rune
    /// transfer in 323262 were both built and hashed with it. So this is a differential test against an independent
    /// implementation with a chain's word behind it, which is the only kind worth having here: a test that checked this
    /// module against a second copy of my own reading of BIP341 would agree with itself and prove nothing.
    ///
    /// The cases are chosen for the parts that are easy to get wrong: more than one input, so `sha_amounts` and
    /// `sha_scriptpubkeys` span several; a non-default version and a non-zero locktime; a sequence of zero and one of
    /// `0xffffffff`; a `vout` of `u32::MAX`; a value above 2^51; and a script long enough to need a two-byte compact size.
    #[test]
    fn the_digest_matches_an_independent_implementation() {
        fn check(psbt_hex: &str, expected: &[&str]) {
            let bytes = hex::decode(psbt_hex).expect("fixture is hex");
            let psbt = Psbt::parse(&bytes).expect("fixture parses");
            assert_eq!(psbt.tx.inputs.len(), expected.len(), "one digest per input");
            for (i, want) in expected.iter().enumerate() {
                let got = taproot_key_path(&psbt, i).expect("every input's value is in the PSBT");
                assert_eq!(hex::encode(got), *want, "input {i} of {psbt_hex}");
            }
        }

        // one input, three outputs, a runestone among them
        check(
            "70736274ff01009c0200000001aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa0100000000fdffffff0300000000000000000a6a5d0714c0a23314c80122020000000000002251201be68a5a028f2601d0e80d468c344ba331d611b96c358b6032e8b4da0547fc11d07e010000000000225120defdea4cdb677750a420fee807eacf21eb9898ae79b9768766e4faa04a2d4a34000000000001012ba086010000000000225120defdea4cdb677750a420fee807eacf21eb9898ae79b9768766e4faa04a2d4a34011720defdea4cdb677750a420fee807eacf21eb9898ae79b9768766e4faa04a2d4a3400000000",
            &[
            "d6b1419e488acccff8cb6509527ca0ef2b9cc22694ff24d887de16c202dcfbc7"
            ],
        );
        // two inputs, so sha_amounts and sha_scriptpubkeys span more than one
        check(
            "70736274ff0100870100000002bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb0000000000ffffffffcccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc0700000000000000000190324101000000002251201be68a5a028f2601d0e80d468c344ba331d611b96c358b6032e8b4da0547fc1100350c000001012b03d9000000000000225120defdea4cdb677750a420fee807eacf21eb9898ae79b9768766e4faa04a2d4a34011720defdea4cdb677750a420fee807eacf21eb9898ae79b9768766e4faa04a2d4a340001012b406f4001000000002251201be68a5a028f2601d0e80d468c344ba331d611b96c358b6032e8b4da0547fc11011720defdea4cdb677750a420fee807eacf21eb9898ae79b9768766e4faa04a2d4a340000",
            &[
            "42fb363873b77839e8d38711ae258364dd525f1294919982e55948d6e3813cd6",
            "e8f7d5665d9cca86a40c847645753f620a6c163005c6a371172f5da7aef2e541"
            ],
        );
        // a big value and a long script, to exercise compact size
        check(
            "70736274ff0100fd69010200000001ddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddffffffff0001000000020000000000000000fd00016a4cfdffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff183c075af0750700225120defdea4cdb677750a420fee807eacf21eb9898ae79b9768766e4faa04a2d4a34000000000001012b0040075af0750700225120defdea4cdb677750a420fee807eacf21eb9898ae79b9768766e4faa04a2d4a34011720defdea4cdb677750a420fee807eacf21eb9898ae79b9768766e4faa04a2d4a34000000",
            &[
            "9fe18d8d02a76b66fa8252a3169abaaed79f431ef02110eeaa35eeb5177fc8d8"
            ],
        );
    }

    /// The message commits to every input's amount, so a venue that lies about what OUR input is worth changes the digest.
    /// This is the property that makes a verified PSBT and a signed digest describe the same transaction, and it is worth
    /// asserting rather than trusting: if it did not hold, `verify` could pass on one transaction and the signature be good
    /// for another.
    #[test]
    fn a_changed_input_amount_changes_the_digest() {
        let cases: &[&str] = &[FIRST_PSBT];
        let bytes = hex::decode(cases[0]).unwrap();
        let mut psbt = Psbt::parse(&bytes).unwrap();
        let before = taproot_key_path(&psbt, 0).unwrap();
        psbt.witness_utxos[0].as_mut().unwrap().value += 1;
        let after = taproot_key_path(&psbt, 0).unwrap();
        assert_ne!(before, after, "one satoshi of difference must move the digest");
    }

    /// And to every output, so a venue that redirects one cannot reuse a digest we already agreed to.
    #[test]
    fn a_changed_output_script_changes_the_digest() {
        let bytes = hex::decode(FIRST_PSBT).unwrap();
        let mut psbt = Psbt::parse(&bytes).unwrap();
        let before = taproot_key_path(&psbt, 0).unwrap();
        let last = psbt.tx.outputs.len() - 1;
        psbt.tx.outputs[last].script[4] ^= 0x01;
        assert_ne!(before, taproot_key_path(&psbt, 0).unwrap(), "a redirected output must move the digest");
    }

    /// An input whose value the PSBT never stated cannot be committed to, so it is refused by name rather than hashed as zero.
    #[test]
    fn an_input_with_no_stated_value_is_refused() {
        let bytes = hex::decode(FIRST_PSBT).unwrap();
        let mut psbt = Psbt::parse(&bytes).unwrap();
        psbt.witness_utxos[0] = None;
        assert_eq!(taproot_key_path(&psbt, 0), Err(SighashError::UnknownInputValue(0)));
        let psbt = Psbt::parse(&bytes).unwrap();
        assert_eq!(taproot_key_path(&psbt, 9), Err(SighashError::NoSuchInput(9)));
    }

    const FIRST_PSBT: &str = "70736274ff01009c0200000001aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa0100000000fdffffff0300000000000000000a6a5d0714c0a23314c80122020000000000002251201be68a5a028f2601d0e80d468c344ba331d611b96c358b6032e8b4da0547fc11d07e010000000000225120defdea4cdb677750a420fee807eacf21eb9898ae79b9768766e4faa04a2d4a34000000000001012ba086010000000000225120defdea4cdb677750a420fee807eacf21eb9898ae79b9768766e4faa04a2d4a34011720defdea4cdb677750a420fee807eacf21eb9898ae79b9768766e4faa04a2d4a3400000000";
}
