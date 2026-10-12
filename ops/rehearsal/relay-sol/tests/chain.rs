//! The depository on mainnet, read-only via the public RPC on 2026-10-09 (fixtures/chain): its PDAs and state, a real deposit,
//! two real refunds of SOL deposits, a real `execute_transfer` and a real solver fill.
mod common;

use common::*;
use rook_relay_sol_spike::*;

fn program() -> [u8; 32] {
    pubkey(DEPOSITORY_PROGRAM).unwrap()
}

#[test]
fn discriminators_are_anchors() {
    assert_eq!(anchor_disc("global", "deposit_native"), IX_DEPOSIT_NATIVE);
    assert_eq!(anchor_disc("global", "deposit_token"), IX_DEPOSIT_TOKEN);
    assert_eq!(anchor_disc("global", "execute_transfer"), IX_EXECUTE_TRANSFER);
    assert_eq!(anchor_disc("event", "DepositEvent"), EV_DEPOSIT);
    assert_eq!(anchor_disc("event", "TransferExecutedEvent"), EV_TRANSFER_EXECUTED);
    assert_eq!(anchor_disc("account", "RelayDepository"), ACC_RELAY_DEPOSITORY);
}

#[test]
fn base58_round_trips() {
    for s in [DEPOSITORY_PROGRAM, SYSTEM_PROGRAM, RELAY_DEPOSITORY, VAULT, ALLOCATOR] {
        assert_eq!(b58_encode(&pubkey(s).unwrap()), s);
    }
    assert_eq!(pubkey(SYSTEM_PROGRAM).unwrap(), [0u8; 32]);
    assert!(pubkey("abc").is_err());
}

#[test]
fn the_state_and_vault_are_the_programs_pdas() {
    let (state, _) = find_program_address(&[b"relay_depository"], &program()).unwrap();
    assert_eq!(b58_encode(&state), RELAY_DEPOSITORY);
    let (vault, bump) = find_program_address(&[b"vault"], &program()).unwrap();
    assert_eq!(b58_encode(&vault), VAULT);
    let (_, _, vault_bump, _) = decode_relay_depository(&account_data("depository-account.json")).unwrap();
    assert_eq!(bump, vault_bump);
}

#[test]
fn the_state_account_names_the_pinned_owner_allocator_and_domain() {
    let acc = chain("depository-account.json");
    assert_eq!(acc["value"]["owner"], DEPOSITORY_PROGRAM);
    let (owner, allocator, _, domain) = decode_relay_depository(&account_data("depository-account.json")).unwrap();
    assert_eq!(b58_encode(&owner), OWNER_AND_UPGRADE_AUTHORITY);
    assert_eq!(b58_encode(&allocator), ALLOCATOR);
    assert_eq!(hex::encode(domain.unwrap()), DOMAIN_SEPARATOR);
}

#[test]
fn the_program_is_upgradeable_by_the_owner_key() {
    let p = chain("program-account.json");
    assert_eq!(p["value"]["owner"], "BPFLoaderUpgradeab1e11111111111111111111111");
    assert_eq!(p["value"]["executable"], true);
    let d = account_data("program-account.json");
    assert_eq!(d[..4], [2, 0, 0, 0]); // UpgradeableLoaderState::Program { programdata_address }
    assert_eq!(b58_encode(&d[4..36]), PROGRAMDATA);
    let (slot, auth) = decode_programdata_head(&account_data("programdata-head.json")).unwrap();
    assert_eq!(slot, PROGRAM_DEPLOY_SLOT);
    assert_eq!(b58_encode(&auth.unwrap()), OWNER_AND_UPGRADE_AUTHORITY);
}

#[test]
fn a_real_deposit_native_its_event_and_the_vault_credit_agree() {
    let tx = chain("tx-deposit-native.json");
    let (sender, ev) = deposit_in(&tx).unwrap().unwrap();
    assert_eq!(b58_encode(&ev.depositor), sender); // sender == depositor in Relay's own flow
    assert_eq!(ev.token, None);
    assert_eq!(lamport_delta(&tx, VAULT), Some(ev.amount as i128));
    assert_eq!(lamport_delta(&tx, &sender).unwrap() + ev.amount as i128 <= 0, true);
}

/// The two SOL refunds Relay's `/requests/v2?status=refund` listed for a Solana origin on 2026-10-09 (`SLIPPAGE`): the deposit
/// went into the vault with id = order id; the refund is a plain System transfer from Relay's solver wallet to the depositor,
/// with the memo `0x<order id>`. Not an `execute_transfer`: the vault is not touched.
#[test]
fn a_real_refund_is_a_solver_transfer_memoed_with_the_order_id() {
    let reqs = chain("relay-requests-sol-refunds.json");
    for (case, deposit, refund) in [(0, "tx-refund-case-deposit.json", "tx-refund-case-refund.json"),
        (1, "tx-refund-case2-deposit.json", "tx-refund-case2-refund.json")] {
        let r = &reqs[case];
        assert_eq!(r["status"], "refund");
        let (dtx, rtx) = (chain(deposit), chain(refund));
        let (depositor, ev) = deposit_in(&dtx).unwrap().unwrap();
        assert_eq!(depositor, r["user"].as_str().unwrap());
        let got = refund_of(&rtx, &depositor, &ev.id).unwrap().expect("recognised as the refund of that order");
        // what came back: the deposit less a few hundred lamports
        assert!(got <= ev.amount && ev.amount - got < 1_000, "{} of {}", got, ev.amount);
        // the payer is the solver, not the depository; nothing of the depository's runs
        let tr = system_transfers_to(&rtx, &depositor).unwrap();
        assert_eq!(tr.len(), 1);
        assert_eq!(tr[0].0, SOLVER_2026_10);
        assert!(tx_instructions(&rtx).unwrap().iter().all(|i| i.program != DEPOSITORY_PROGRAM));
        assert_eq!(lamport_delta(&rtx, VAULT), None);
        // a memo with another id, or a credit to someone else, is not this refund
        let mut other = ev.id;
        other[0] ^= 1;
        assert_eq!(refund_of(&rtx, &depositor, &other).unwrap(), None);
        assert_eq!(refund_of(&rtx, SOLVER_2026_10, &ev.id).unwrap(), None);
        // the refund comes in the same minute as the deposit
        assert!(rtx["blockTime"].as_i64().unwrap() - dtx["blockTime"].as_i64().unwrap() < 120);
    }
}

/// `execute_transfer` is the allocator-signed release from the vault. The one saved is the solver withdrawing a token deposit
/// to itself (recipient == executor == the solver): Relay's settlement of deposits, not a payout to a user and not tied to any
/// order id (the request carries none).
#[test]
fn a_real_execute_transfer_is_the_solver_settling_and_names_no_order() {
    let tx = chain("tx-execute-transfer.json");
    let ixs = tx_instructions(&tx).unwrap();
    let ix = ixs.iter().find(|i| i.program == DEPOSITORY_PROGRAM).unwrap();
    let req = decode_execute_transfer(&ix.data).unwrap();
    assert_eq!(hex::encode(req.domain), DOMAIN_SEPARATOR);
    assert_eq!(b58_encode(&req.vault_address), VAULT);
    assert_eq!(b58_encode(&req.recipient), SOLVER_2026_10);
    assert_eq!(ix.accounts[1], SOLVER_2026_10); // executor
    assert_eq!(ix.accounts[2], SOLVER_2026_10); // recipient
    assert!(req.token.is_some());
    assert_eq!(encode_transfer_request(&req), ix.data[8..].to_vec());
    // the allocator's signature rides in an Ed25519SigVerify instruction just before it, over sha256(borsh(request))
    let sig = ixs.iter().find(|i| i.program == "Ed25519SigVerify111111111111111111111111111").unwrap();
    let msg_hash: [u8; 32] = <sha2::Sha256 as sha2::Digest>::digest(encode_transfer_request(&req)).into();
    let off = |i: usize| u16::from_le_bytes([sig.data[i], sig.data[i + 1]]) as usize;
    let (pk_at, msg_at, msg_len) = (off(6), off(10), off(12));
    assert_eq!(b58_encode(&sig.data[pk_at..pk_at + 32]), ALLOCATOR);
    assert_eq!(sig.data[msg_at..msg_at + msg_len], msg_hash);
    // the event repeats the request; its id is the used_request account, PDA ["used_request", sha256(borsh(request))]
    let evs: Vec<_> = program_data(&tx, DEPOSITORY_PROGRAM).iter().filter_map(|b| decode_transfer_executed(b)).collect();
    assert_eq!(evs.len(), 1);
    let (ereq, executor, id) = &evs[0];
    assert_eq!(*ereq, req);
    assert_eq!(b58_encode(executor), SOLVER_2026_10);
    assert_eq!(b58_encode(id), ix.accounts[7]);
    let (pda, _) = find_program_address(&[b"used_request", &msg_hash], &program()).unwrap();
    assert_eq!(b58_encode(&pda), ix.accounts[7]);
}

/// A fill on Solana: plain transfers from the solver with two memos, the order id and Relay's request id (0x1791… prefix).
#[test]
fn a_real_solver_fill_carries_order_and_request_memos() {
    let tx = chain("tx-solver-fill-memo.json");
    let m = memos(&tx).unwrap();
    assert_eq!(m.len(), 2);
    assert!(m.iter().all(|x| x.len() == 66 && x.starts_with("0x")));
    assert!(m[1].starts_with("0x1791"));
    // Relay's own record of that request (/requests/v2?id=<the second memo>): a 4663 → Solana fill whose orderId is the first memo
    let r = chain("relay-request-solver-fill.json");
    assert_eq!(r["id"].as_str().unwrap(), m[1]);
    assert_eq!(r["data"]["outTxs"][0]["chainId"], RELAY_SOLANA);
    assert_eq!(r["data"]["inTxs"][0]["chainId"], 4663);
    let order_id = serde_json::to_string(&r).unwrap().split("\"orderId\":\"").nth(1).unwrap()[..66].to_string();
    assert_eq!(order_id, m[0]);
    // the payee is credited with SPL tokens here (a token fill); a SOL fill is the same shape with a System transfer
    assert_eq!(tx_instructions(&tx).unwrap().iter().filter(|i| i.program == "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA").count(), 1);
}
