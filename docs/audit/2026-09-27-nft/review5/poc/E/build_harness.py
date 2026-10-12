#!/usr/bin/env python3
"""Round 5, part E: builds tests/r5_e.rs in a PRIVATE copy of autobuyer/canister-rs from the CURRENT tests/audit_nft_r4.rs.

The harness is audit_nft_r4.rs's fake world up to its first round-4 test (lines 1-750), plus its E4-H-1 helpers (launch_limits,
azuki_evidence, mint_demand), with five switches added to the fake (nothing else in it changes):

  mint_revert   the next N wrapper mints (mint / mintDeposit) revert on 4663 (nothing minted)
  mint_log      every wrapper mint that succeeded: (wrapper, time)
  hidden        home transactions every provider fails to show (home_seen: NotYet), though they mined
  item_of_err   the next N reads of a wrapper's itemOf fail (a transient read error)
  estimate      home_sign refuses a mint whose token the wrapper already holds, as the real EthNet::home_sign's
                eth_estimateGas does ("would not succeed now: ... AlreadyHeld")

then appends r5_e.tests.rs. Usage: build_harness.py <crate dir> <this dir>
"""
import sys, pathlib

crate, here = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
src = (crate / "tests/audit_nft_r4.rs").read_text().split("\n")
head = src[:750]
helpers = "\n".join(src[885:914])  # launch_limits, azuki_evidence, mint_demand
h = "\n".join(head)

def sub(old, new, count=1):
    global h
    assert h.count(old) == count, (old, h.count(old))
    h = h.replace(old, new)

sub("    payouts: HashMap<(bool, [u8; 32]), ef::PayoutTx>,\n}",
    "    payouts: HashMap<(bool, [u8; 32]), ef::PayoutTx>,\n"
    "    // ---- review round 5, part E ----\n"
    "    mint_revert: u32,\n    mint_log: Vec<([u8; 20], u64)>,\n    hidden: Vec<[u8; 32]>,\n    item_of_err: u32,\n}")
sub("            sale_moves: HashMap::new(), payouts: HashMap::new(),\n",
    "            sale_moves: HashMap::new(), payouts: HashMap::new(),\n"
    "            mint_revert: 0, mint_log: vec![], hidden: vec![], item_of_err: 0,\n")
sub("        match (to, sel(d)) {\n            (WRAPPER, s) if s == en::MINT || s == en::MINT_DEPOSIT =>",
    "        if (to == WRAPPER || to == WRAPPER2) && (sel(d) == en::MINT || sel(d) == en::MINT_DEPOSIT) {\n"
    "            if self.mint_revert > 0 {\n                self.mint_revert -= 1;\n                return false; // reverted: nothing minted\n            }\n"
    "            self.mint_log.push((to, now));\n        }\n"
    "        match (to, sel(d)) {\n            (WRAPPER, s) if s == en::MINT || s == en::MINT_DEPOSIT =>")
sub("    async fn home_sign(&mut self, to: [u8; 20], value: u128, data: &[u8]) -> Result<(Vec<u8>, [u8; 32], u64), String> {\n",
    "    async fn home_sign(&mut self, to: [u8; 20], value: u128, data: &[u8]) -> Result<(Vec<u8>, [u8; 32], u64), String> {\n"
    "        // round 5: the real home_sign estimates first; a mint of a token the wrapper already holds reverts (AlreadyHeld / AlreadyMinted)\n"
    "        if (to == WRAPPER || to == WRAPPER2) && (sel(data) == en::MINT || sel(data) == en::MINT_DEPOSIT) {\n"
    "            let t: Word = data[4..36].try_into().unwrap();\n"
    "            if (if to == WRAPPER { &self.items } else { &self.items2 }).contains(&t) {\n"
    "                return Err(\"would not succeed now: execution reverted: AlreadyHeld()\".into());\n            }\n        }\n")
sub("        match self.home_sent.get(&h) {\n            Some((ok, _)) => EvmView::Mined { success: *ok },",
    "        if self.hidden.contains(&h) {\n            return EvmView::NotYet;\n        }\n"
    "        match self.home_sent.get(&h) {\n            Some((ok, _)) => EvmView::Mined { success: *ok },")
sub("        Ok(match to {\n            WRAPPER => Fake::wrapper_read(&self.tickets, &self.items, d),",
    "        if (to == WRAPPER || to == WRAPPER2) && sel(d) == en::ITEM_OF && self.item_of_err > 0 {\n"
    "            self.item_of_err -= 1;\n            return Err(\"chain 4663: eth_call: IC0406: SysTransient: Timeout expired\".into());\n        }\n"
    "        Ok(match to {\n            WRAPPER => Fake::wrapper_read(&self.tickets, &self.items, d),")

h = h.replace("//! Fix round 4 of the 2026-09-27 NFT review", "//! Review round 5, part E (built by review5/poc/E/build_harness.py from) fix round 4 of the 2026-09-27 NFT review", 1)
out = h + "\n" + helpers + "\n\n" + (here / "r5_e.tests.rs").read_text()
(crate / "tests/r5_e.rs").write_text(out)
print("wrote", crate / "tests/r5_e.rs", len(out.split("\n")), "lines")
