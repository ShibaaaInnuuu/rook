//! Sending a signed Bitcoin transaction, and why the send is never the source of truth (`docs/v2/btc-broadcast.md`).
//!
//! The Bitcoin half of B4. A replicated `POST /tx` cannot reach consensus: the explorer accepts the first copy and refuses
//! every later one, the replies differ, and the canister is told the send failed about a transaction that was sent. So the
//! send is made by **one** node and only reported; what is believed is the replicated read-back of a txid the canister
//! computed itself from the bytes it holds.
//!
//! This module is pure: no network, no replica. It parses a signed transaction far enough to know it is one and to compute
//! its txid, classifies an esplora `POST /tx` reply, and projects a status reply onto what the buyer acts on.

use sha2::{Digest, Sha256};

/// What an explorer said to one send. A report, not a fact.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub enum BtcSendOutcome {
    /// It answered with our txid.
    Accepted,
    /// It answered with a txid that is not the one we computed. Not to be believed about anything.
    WrongTxid(String),
    /// "already in block chain" / "outputs already in utxo set": this very transaction is confirmed.
    AlreadyConfirmed,
    /// Already in its mempool: as good as accepted.
    AlreadyInMempool,
    /// An input is missing or spent. Either this transaction confirmed and its outputs were spent since, or ANOTHER
    /// transaction spent the same input. Only the read-back can say which, so this is never read as success.
    MissingInputs,
    /// Any other refusal, with the explorer's message (shortened).
    Refused(String),
    /// Not an answer: an outcall failure, an HTML error page.
    Unreadable(String),
}

impl BtcSendOutcome {
    /// Whether this reply says the explorer has the transaction. Still only a report.
    pub fn has_it(&self) -> bool {
        matches!(self, Self::Accepted | Self::AlreadyConfirmed | Self::AlreadyInMempool)
    }
}

/// Classify one esplora `POST /tx` reply: the txid as plain text on success, `sendrawtransaction RPC error …` otherwise.
/// `our_txid` is the displayed (reversed) hex the canister computed.
pub fn classify(status: u16, body: &[u8], our_txid: &str) -> BtcSendOutcome {
    let text = String::from_utf8_lossy(body).trim().to_string();
    let short = |s: &str| s.chars().take(200).collect::<String>();
    if (200..300).contains(&status) {
        let t = text.to_ascii_lowercase();
        return if t == our_txid.to_ascii_lowercase() {
            BtcSendOutcome::Accepted
        } else if t.len() == 64 && t.bytes().all(|b| b.is_ascii_hexdigit()) {
            BtcSendOutcome::WrongTxid(t)
        } else {
            BtcSendOutcome::Unreadable(format!("HTTP {status} but not a txid: {}", short(&text)))
        };
    }
    let m = text.to_ascii_lowercase();
    if !m.contains("sendrawtransaction") && !m.contains("rpc error") {
        return BtcSendOutcome::Unreadable(format!("HTTP {status}: {}", short(&text)));
    }
    // Bitcoin Core's own reason strings, as mempool.space and blockstream.info pass them through
    if m.contains("already in block chain") || m.contains("outputs already in utxo set") {
        BtcSendOutcome::AlreadyConfirmed
    } else if m.contains("txn-already-in-mempool") || m.contains("txn-already-known") || m.contains("already in mempool") {
        BtcSendOutcome::AlreadyInMempool
    } else if m.contains("missingorspent") || m.contains("missing-inputs") || m.contains("missing inputs") {
        BtcSendOutcome::MissingInputs
    } else {
        BtcSendOutcome::Refused(short(&text))
    }
}

/// One outpoint: the txid in internal (wire) byte order, and the output index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutPoint {
    pub txid: [u8; 32],
    pub vout: u32,
}

/// A signed transaction, read far enough to be sure it is one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedTx {
    /// The txid in internal byte order: double SHA-256 of the non-witness serialisation.
    pub txid: [u8; 32],
    pub inputs: Vec<OutPoint>,
    /// (value in sats, scriptPubKey)
    pub outputs: Vec<(u64, Vec<u8>)>,
    pub segwit: bool,
}

impl SignedTx {
    /// The txid as Bitcoin displays it: the bytes reversed, in hex.
    pub fn txid_hex(&self) -> String {
        display(&self.txid)
    }
}

/// Internal byte order to the displayed hex.
pub fn display(txid: &[u8; 32]) -> String {
    let mut r = *txid;
    r.reverse();
    hex::encode(r)
}

/// Displayed hex back to internal byte order.
pub fn from_display(s: &str) -> Result<[u8; 32], String> {
    let s = s.trim().to_ascii_lowercase();
    let mut b: [u8; 32] = hex::decode(&s)
        .map_err(|_| "a txid is 64 hex digits".to_string())?
        .try_into()
        .map_err(|_| "a txid is 64 hex digits".to_string())?;
    b.reverse();
    Ok(b)
}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.i.checked_add(n).ok_or("length overflows")?;
        if end > self.b.len() {
            return Err(format!("the transaction ends early at byte {}", self.b.len()));
        }
        let s = &self.b[self.i..end];
        self.i = end;
        Ok(s)
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn compact(&mut self) -> Result<u64, String> {
        let first = self.take(1)?[0];
        Ok(match first {
            0xfd => u16::from_le_bytes(self.take(2)?.try_into().unwrap()) as u64,
            0xfe => self.u32()? as u64,
            0xff => self.u64()?,
            n => n as u64,
        })
    }
    /// A count or a length, bounded by what is left: a claimed length past the end is refused before anything is allocated.
    fn len(&mut self) -> Result<usize, String> {
        let n = self.compact()?;
        if n > (self.b.len() - self.i) as u64 {
            return Err(format!("a length of {n} runs past the end of the transaction"));
        }
        Ok(n as usize)
    }
}

/// Parse a signed transaction, segwit or not. Refuses anything that does not parse to the last byte, and a transaction with
/// no input or no output.
pub fn parse_signed(raw: &[u8]) -> Result<SignedTx, String> {
    let mut r = Reader { b: raw, i: 0 };
    let version = r.take(4)?;
    let segwit = raw.len() > 6 && raw[4] == 0x00 && raw[5] == 0x01;
    if segwit {
        r.take(2)?;
    }
    let body_start = r.i;
    let n_in = r.len()?;
    if n_in == 0 {
        return Err("a transaction with no input".into());
    }
    let mut inputs = Vec::with_capacity(n_in);
    for _ in 0..n_in {
        let txid: [u8; 32] = r.take(32)?.try_into().unwrap();
        let vout = r.u32()?;
        let script = r.len()?;
        r.take(script)?;
        r.u32()?; // sequence
        inputs.push(OutPoint { txid, vout });
    }
    let n_out = r.len()?;
    if n_out == 0 {
        return Err("a transaction with no output".into());
    }
    let mut outputs = Vec::with_capacity(n_out);
    for _ in 0..n_out {
        let value = r.u64()?;
        let script = r.len()?;
        outputs.push((value, r.take(script)?.to_vec()));
    }
    let body_end = r.i;
    if segwit {
        for _ in 0..n_in {
            let items = r.len()?;
            for _ in 0..items {
                let n = r.len()?;
                r.take(n)?;
            }
        }
    }
    let locktime = r.take(4)?;
    if r.i != raw.len() {
        return Err(format!("{} bytes after the locktime: not one transaction", raw.len() - r.i));
    }
    // the txid commits to the non-witness serialisation: version ‖ inputs ‖ outputs ‖ locktime
    let mut h = Sha256::new();
    h.update(version);
    h.update(&raw[body_start..body_end]);
    h.update(locktime);
    let once = h.finalize();
    let txid: [u8; 32] = Sha256::digest(once).into();
    Ok(SignedTx { txid, inputs, outputs, segwit })
}

/// What an explorer's `GET /tx/{txid}/status` says, reduced to what the buyer acts on. Explorers must agree on exactly these;
/// `block_time` and anything else is presentation. A confirmed status without a height or a hash is not a status.
pub fn status_view(body: &[u8]) -> Result<(bool, Option<u64>, Option<String>), String> {
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|_| "a status reply that is not JSON".to_string())?;
    let confirmed = v.get("confirmed").and_then(|c| c.as_bool()).ok_or("a status reply without `confirmed`")?;
    if !confirmed {
        return Ok((false, None, None));
    }
    let height = v.get("block_height").and_then(|h| h.as_u64()).ok_or("confirmed, but no block_height")?;
    let hash = v.get("block_hash").and_then(|h| h.as_str()).ok_or("confirmed, but no block_hash")?.to_ascii_lowercase();
    if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("confirmed, but block_hash is not 64 hex digits".into());
    }
    Ok((true, Some(height), Some(hash)))
}

/// The block hash every explorer must return at a fixed height before anything is sent or believed on `network`. Genesis
/// cannot do this job: every signet shares one genesis block, and mutinynet (a different signet) was once configured here as
/// a "second signet explorer" (`docs/v2/btc-broadcast.md`).
///
/// Mainnet's (Phase-1 K2, launch gate C7): block 900,000, mined 2025-06-06, read identically from mempool.space,
/// blockstream.info, btcscan.org, BlockCypher and blockchain.com on 2026-09-25, and tied here to its header by
/// [`tests::the_mainnet_checkpoint_is_the_double_sha_of_a_header_whose_work_is_real`] (`docs/deploy/canister-mainnet.md`, "The
/// published C7 check"). An alias such as `"bitcoin"` has none: the install spells it `"mainnet"` or is refused.
pub fn checkpoint(network: &str) -> Option<(u64, &'static str)> {
    match network {
        "signet" => Some((300_000, "000000073002e4e1de008de89ee41db4baf8734c0f4e5ba9447bb0f1a301b02c")),
        "testnet4" => Some((50_000, "00000000e2c8c94ba126169a88997233f07a9769e2b009fb10cad0e893eff2cb")),
        "mainnet" => Some((900_000, "000000000000000000010538edbfd2d5b809a33dd83f284aeea41c6d0d96968a")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Mined on signet by this project (autobuyer/.signet-rehearsal.json, .runes-rehearsal.json): the node computed these txids.
    const SPEND: &str = "020000000001015daa1437c6367e02c51205350f549442b9ef63426d4e3c84d71e2fbdec2cdcea0100000000ffffffff016707000000000000225120efafa8031634fa887429957024458a793e5d45e569a2696a24aa9fdd0352071d0140957664727df1e3ed24bab37d3f79a8c1f718388221552d6a1af156253b9c3315c407613b24876586aab086ff41bda71ac4011865a3e9b9e9a273b9714d4bccf600000000";
    const SPEND_TXID: &str = "3d3ed2734b97895f5a4d32f8b74ebbcc8273803df65c8bbc0fd9832dd40f7f9f";
    const MINT: &str = "020000000001010d190a160b883404c0fdfb1f13c4c8ef1eedd2b7c7427459f987c4fb5ec5b8801b05000000ffffffff032202000000000000225120efafa8031634fa887429957024458a793e5d45e569a2696a24aa9fdd0352071d00000000000000000b6a5d081484c80b140f16007e37030000000000225120efafa8031634fa887429957024458a793e5d45e569a2696a24aa9fdd0352071d01407eee7a2c74b3ae6a219615a9d23789691ee7ce845f6b814f6d5cb94b35697389a698889d49ec799e05f0855f5db10789f1b484bc9074a1e605514d5b5630811800000000";
    const MINT_TXID: &str = "deee13a785e33a743b7c85e46dcbc7e275eb1d9eb265f123ba21423dbfa5a66c";
    const TRANSFER: &str = "020000000001026ca6a5bf3d4221ba23f165b29e1deb75e2c7cb6de4857c3b743ae385a713eede0000000000ffffffff6ca6a5bf3d4221ba23f165b29e1deb75e2c7cb6de4857c3b743ae385a713eede0200000000ffffffff032202000000000000225120efafa8031634fa887429957024458a793e5d45e569a2696a24aa9fdd0352071d00000000000000000c6a5d0916020084c80b0f01009d36030000000000225120efafa8031634fa887429957024458a793e5d45e569a2696a24aa9fdd0352071d0140a8a4f9e49cdf6f3aa826fbb31257a9e5430aea926e7fddd4d393b8e1ea3b6ae550db21fc1fa058991552ccd31c980f40ff02b8c4608d2715538024a3ae308ad40140502241686cce768206a2952257f7a8788b4c2ba0f3d4648b49b28fc402461e234e3c22c6c557df110ef4eef25651586d408d9c72ca8c5646d59b605fa0c1ecb200000000";
    const TRANSFER_TXID: &str = "8730ef716246308292ba4710a5363baa6366b8191a7bb7ee339cdc1e55040bf9";
    // Block 170: the first payment in Bitcoin, pre-segwit, a scriptSig and two outputs.
    const B170: &str = "0100000001c997a5e56e104102fa209c6a852dd90660a20b2d9c352423edce25857fcd3704000000004847304402204e45e16932b8af514961a1d3a1a25fdf3f4f7732e9d624c6c61548ab5fb8cd410220181522ec8eca07de4860a4acdd12909d831cc56cbbac4622082221a8768d1d0901ffffffff0200ca9a3b00000000434104ae1a62fe09c5f51b13905f07f06b99a2f7159b2225f374cd378d71302fa28414e7aab37397f554a7df5f142c21c1b7303b8a0626f1baded5c72a704f7e6cd84cac00286bee0000000043410411db93e1dcdb8a016b49840f8c53bc1eb68a382e97b1482ecad7b148a6909a5cb2e0eaddfb84ccf9744464f82e160bfa9b8b64f9d4c03f999b8643f656b412a3ac00000000";
    const B170_TXID: &str = "f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16";

    #[test]
    fn the_txid_is_computed_here_and_matches_what_the_chain_computed() {
        for (raw, want, n_in, n_out, segwit) in [
            (SPEND, SPEND_TXID, 1, 1, true),
            (MINT, MINT_TXID, 1, 3, true),
            (TRANSFER, TRANSFER_TXID, 2, 3, true),
            (B170, B170_TXID, 1, 2, false),
        ] {
            let tx = parse_signed(&hex::decode(raw).unwrap()).unwrap();
            assert_eq!(tx.txid_hex(), want, "the witness is not in the txid, and the display is reversed");
            assert_eq!((tx.inputs.len(), tx.outputs.len(), tx.segwit), (n_in, n_out, segwit));
            assert_eq!(from_display(want).unwrap(), tx.txid);
        }
        // the transfer's inputs are the mint's outputs 0 and 2: outpoints are read in wire order
        let t = parse_signed(&hex::decode(TRANSFER).unwrap()).unwrap();
        assert_eq!(t.inputs.iter().map(|i| (display(&i.txid), i.vout)).collect::<Vec<_>>(),
            vec![(MINT_TXID.to_string(), 0), (MINT_TXID.to_string(), 2)]);
        assert_eq!(t.outputs[1], (0, hex::decode("6a5d0916020084c80b0f0100").unwrap()), "the runestone output, byte for byte");
    }

    #[test]
    fn anything_that_is_not_exactly_one_transaction_is_refused() {
        let raw = hex::decode(SPEND).unwrap();
        assert!(parse_signed(&raw[..raw.len() - 1]).unwrap_err().contains("ends early"));
        let mut more = raw.clone();
        more.push(0);
        assert!(parse_signed(&more).unwrap_err().contains("after the locktime"));
        assert!(parse_signed(&[]).is_err());
        // a length prefix claiming more than exists is refused before anything is allocated
        let mut huge = hex::decode("0200000001").unwrap();
        huge.extend_from_slice(&[0u8; 36]);
        huge.extend_from_slice(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f]);
        assert!(parse_signed(&huge).unwrap_err().contains("runs past the end"));
        assert!(parse_signed(&hex::decode("020000000000000000").unwrap()).is_err(), "no input");
        assert!(from_display("abc").is_err());
    }

    #[test]
    fn the_replies_the_explorers_actually_give() {
        let e = |s: &str| classify(400, s.as_bytes(), SPEND_TXID);
        assert_eq!(classify(200, SPEND_TXID.as_bytes(), SPEND_TXID), BtcSendOutcome::Accepted);
        assert_eq!(classify(200, format!("{}\n", SPEND_TXID.to_uppercase()).as_bytes(), SPEND_TXID), BtcSendOutcome::Accepted);
        assert_eq!(classify(200, MINT_TXID.as_bytes(), SPEND_TXID), BtcSendOutcome::WrongTxid(MINT_TXID.into()));
        // measured 2026-09-23: resending the confirmed rehearsal spend, whose outputs are since spent, to both explorers
        assert_eq!(e(r#"sendrawtransaction RPC error: {"code":-25,"message":"bad-txns-inputs-missingorspent"}"#), BtcSendOutcome::MissingInputs);
        assert_eq!(e("sendrawtransaction RPC error -25: bad-txns-inputs-missingorspent"), BtcSendOutcome::MissingInputs);
        assert!(!BtcSendOutcome::MissingInputs.has_it(), "a spent input may be someone else's spend: only the read-back decides");
        assert_eq!(e(r#"sendrawtransaction RPC error: {"code":-27,"message":"Transaction already in block chain"}"#), BtcSendOutcome::AlreadyConfirmed);
        assert_eq!(e(r#"sendrawtransaction RPC error: {"code":-27,"message":"Transaction outputs already in utxo set"}"#), BtcSendOutcome::AlreadyConfirmed);
        assert_eq!(e(r#"sendrawtransaction RPC error: {"code":-26,"message":"txn-already-in-mempool"}"#), BtcSendOutcome::AlreadyInMempool);
        assert!(matches!(e(r#"sendrawtransaction RPC error: {"code":-26,"message":"min relay fee not met"}"#), BtcSendOutcome::Refused(m) if m.contains("min relay")));
        assert!(matches!(e(r#"sendrawtransaction RPC error: {"code":-22,"message":"TX decode failed. Make sure the tx has at least one input."}"#),
            BtcSendOutcome::Refused(_)));
        assert!(matches!(classify(502, b"<html>Bad Gateway</html>", SPEND_TXID), BtcSendOutcome::Unreadable(m) if m.starts_with("HTTP 502")));
        assert!(matches!(classify(200, b"ok", SPEND_TXID), BtcSendOutcome::Unreadable(_)));
    }

    #[test]
    fn a_status_is_what_the_buyer_acts_on_and_nothing_else() {
        let c = br#"{"confirmed":true,"block_height":323262,"block_hash":"0000000044578d8ede9e9a374539fb2fae0157da59361817d5bccfb90c189dcd","block_time":1790101493}"#;
        let other_time = br#"{"confirmed":true,"block_height":323262,"block_hash":"0000000044578D8EDE9E9A374539FB2FAE0157DA59361817D5BCCFB90C189DCD","block_time":1}"#;
        assert_eq!(status_view(c).unwrap(), status_view(other_time).unwrap(), "block_time and case are presentation");
        assert_eq!(status_view(c).unwrap().1, Some(323262));
        assert_eq!(status_view(br#"{"confirmed":false}"#).unwrap(), (false, None, None));
        assert!(status_view(br#"{"confirmed":true,"block_height":1}"#).is_err(), "confirmed without a hash is not a status");
        assert!(status_view(b"Transaction not found").is_err());
        assert!(status_view(br#"{"block_height":1}"#).is_err());
    }

    #[test]
    fn each_network_has_a_checkpoint_and_nothing_else_does() {
        assert_eq!(checkpoint("signet").unwrap().0, 300_000);
        assert!(checkpoint("testnet4").is_some());
        assert_eq!(checkpoint("mainnet"), Some((900_000, "000000000000000000010538edbfd2d5b809a33dd83f284aeea41c6d0d96968a")));
        for alias in ["bitcoin", "Mainnet", "main", "mainnet ", "", "testnet", "regtest"] {
            assert!(checkpoint(alias).is_none(), "{alias:?} has no checkpoint: nothing is sent or believed there");
        }
    }

    /// Block 900,000's header, as mempool.space and blockstream.info served it on 2026-09-25 (`/api/block/<hash>/header`).
    const HEADER_900000: &str = "00a0ab20247d4d9f582f9750344cdf62c46d81d046be960340960100000000000000000070f96945530651135839d8adc3f40e595118ec74c7ad81a3d17bb022e554fb0c937f4268743702177ad05f92";

    /// The checkpoint in the wasm is not only the explorers' word: it is the double SHA-256 of this header, whose proof of work
    /// is real at mainnet difficulty (a test network or another chain cannot produce it), and whose parent and time are the
    /// ones every explorer reported (docs/deploy/canister-mainnet.md, "The published C7 check").
    #[test]
    fn the_mainnet_checkpoint_is_the_double_sha_of_a_header_whose_work_is_real() {
        let h = hex::decode(HEADER_900000).unwrap();
        assert_eq!(h.len(), 80);
        let hash: [u8; 32] = Sha256::digest(Sha256::digest(&h)).into();
        let (height, want) = checkpoint("mainnet").unwrap();
        assert_eq!((height, display(&hash)), (900_000, want.to_string()));
        // the work: the hash, read as a little-endian number, is under the target the header's bits encode
        let bits = u32::from_le_bytes(h[72..76].try_into().unwrap());
        assert_eq!(bits, 0x1702_3774);
        let (exp, mant) = ((bits >> 24) as usize, bits & 0x00ff_ffff);
        let mut target = [0u8; 32]; // big-endian
        let at = 32 - exp;
        target[at..at + 3].copy_from_slice(&mant.to_be_bytes()[1..]);
        let mut be = hash;
        be.reverse();
        assert!(be < target, "the header's hash meets its own target");
        assert!(be[..9].iter().all(|b| *b == 0), "72 leading zero bits: mainnet-scale work, which no test network has");
        // the parent and the time every explorer reported
        let mut prev: [u8; 32] = h[4..36].try_into().unwrap();
        prev.reverse();
        assert_eq!(hex::encode(prev), "0000000000000000000196400396be46d0816dc462df4c3450972f589f4d7d24");
        assert_eq!(u32::from_le_bytes(h[68..72].try_into().unwrap()), 1_749_188_499, "2025-06-06 05:41:39 UTC");
    }
}
