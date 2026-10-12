import { createHash } from "node:crypto";
import { crc32 } from "node:zlib";
import { Principal } from "@icp-sdk/core/principal";
import { encodeFunctionData, parseAbi, toFunctionSelector, type Hex } from "viem";

/**
 * docs/v7/cycles-from-fees.md §2 and §4.2 (v1, the fueler). Every destination the fueler may ever pay or call, pinned here and
 * nowhere else. Nothing is read from a mutable source: a live answer (the minter's helper address, the pool's tokens) is only
 * compared with these, and a mismatch halts the run and pages. Read live on 2026-10-09 (get_minter_info, the pool's metadata).
 */

/** The IC side. */
export const IC = {
  /** The cycles minting canister: mints cycles from ICP only (`notify_top_up`). */
  CMC: "rkp4c-7iaaa-aaaaa-aaaca-cai",
  /** The ICP ledger (legacy `transfer` with the TPUP memo is what `dfx cycles top-up` and the CMC's own docs use). */
  ICP_LEDGER: "ryjl3-tyaaa-aaaaa-aaaba-cai",
  /** The ckETH/ckERC20 minter and the ckETH ledger (18 decimals, fee 2e12 wei). */
  CKETH_MINTER: "sv3dd-oaaaa-aaaar-qacoa-cai",
  CKETH_LEDGER: "ss2fx-dyaaa-aaaar-qacoq-cai",
  /** ICPSwap's ICP/ckETH pool, fee tier 0.3 %: token0 = ICP, token1 = ckETH, so ckETH → ICP is `zeroForOne = false`. */
  POOL_ICP_CKETH: "angxa-baaaa-aaaag-qcvnq-cai",
} as const;

export const ICP_FEE_E8S = 10_000n;
export const CKETH_FEE_WEI = 2_000_000_000_000n;
/** The CMC's top-up memo, "TPUP" as a u64 (0x50555054). */
export const TPUP_MEMO = 0x50555054n;

/** The Ethereum side: the ckETH helper that takes `depositEth(bytes32 principal, bytes32 subaccount)` (get_minter_info's
 *  `deposit_with_subaccount_helper_contract_address`, verified again every run). */
export const CKETH_HELPER = "0x18901044688d3756c35ed2b36d93e6a5b8e00e68";

export const ROBINHOOD = 4663;
export const ETHEREUM = 1;

export const ABI = parseAbi([
  "function claimPlatformFees()",
  "function allocate()",
  "function unallocated() view returns (uint256)",
  "function ops() view returns (address)",
  "function totalToOps() view returns (uint256)",
  "function platformOwed() view returns (uint256)",
  "function depositEth(bytes32 principal, bytes32 subaccount) payable",
]);

export const SEL = {
  claimPlatformFees: toFunctionSelector("function claimPlatformFees()"),
  allocate: toFunctionSelector("function allocate()"),
  depositEth: toFunctionSelector("function depositEth(bytes32,bytes32)"),
  depositNative: "0x49290c1c" as Hex,
};

export const ZERO32 = `0x${"00".repeat(32)}` as Hex;

/**
 * The 32-byte form of a principal that the ckETH minter reads back from the helper's event, and that the CMC uses as a top-up
 * subaccount: its length, then its bytes, zero-padded (dfinity/ic, rs/ethereum/cketh/minter/src/eth_logs/mod.rs `encode_principal`;
 * the ledger's `Subaccount::from(&PrincipalId)` is the same layout). A wrong encoding loses the deposit, so it is tested against
 * vectors dfx printed.
 */
export function principal32(p: Principal | string): Uint8Array {
  const bytes = (typeof p === "string" ? Principal.fromText(p) : p).toUint8Array();
  if (bytes.length > 29) throw new Error(`a principal of ${bytes.length} bytes does not fit 32 with its length`);
  const out = new Uint8Array(32);
  out[0] = bytes.length;
  out.set(bytes, 1);
  return out;
}
export const principal32Hex = (p: Principal | string): Hex => `0x${Buffer.from(principal32(p)).toString("hex")}` as Hex;

/** The ICP ledger's account identifier: CRC32 (big-endian) ‖ SHA-224("\x0Aaccount-id" ‖ owner ‖ subaccount). */
export function accountId(owner: Principal | string, subaccount: Uint8Array = new Uint8Array(32)): Uint8Array {
  if (subaccount.length !== 32) throw new Error("a subaccount is 32 bytes");
  const o = (typeof owner === "string" ? Principal.fromText(owner) : owner).toUint8Array();
  const h = createHash("sha224").update(Buffer.from("\x0Aaccount-id")).update(o).update(subaccount).digest();
  const c = Buffer.alloc(4);
  c.writeUInt32BE(crc32(h) >>> 0);
  return Uint8Array.from(Buffer.concat([c, h]));
}

/** Where a top-up of `canister` is paid: the CMC's account under the canister's own subaccount. */
export const topUpAccount = (canister: string): Uint8Array => accountId(IC.CMC, principal32(canister));

/** The calldata of the only Ethereum call the fueler makes. */
export const depositEthData = (principal: Principal | string): Hex =>
  encodeFunctionData({ abi: ABI, functionName: "depositEth", args: [principal32Hex(principal), ZERO32] });
