# deployments/launch/btc: the Bitcoin collections of the launch

**Bitcoin Frogs** (`bitcoin-frogs`), the collection the Bitcoin side (wFROG) was built for, chosen 2026-10-09 for ordinals at launch.

`bitcoin-frogs-members.json` is the collection's membership: its 10,000 inscription ids, **ordered by inscription number**
(ascending), so anyone can rebuild it. Taken 2026-10-09 from two independent sources that agree exactly (10,000 ids each, all on
both, none on one only):

- Ordinals Wallet, `GET https://turbo.ordinalswallet.com/collection/bitcoin-frogs/inscriptions` (keyless; `total_supply` 10,000);
- UniSat's collection index, `GET https://open-api.unisat.io/v1/collection-indexer/collection/bitcoin-frogs/items` (keyed).

Its membership root, built as `ops/src/frogmath.ts` `proofFor` and `script/Deploy.s.sol` build it (leaves in list order,
`keccak256(keccak256(id))`, sorted pairs, an odd node moves up):

```
0x801f71bbb4c7ee37759a3b23da76bc3dd5ec132c0dd10edefe3dc86acfae418e
```

It is the root `list-auto.sh` (`listInscriptionsAuto`) gives the Bitcoin listing, and the file the runner proves membership from
(`RUNNER_OW_MEMBERS`, baked into the runner image at `/app/deployments/launch/btc/bitcoin-frogs-members.json`). Rebuild and compare:

```sh
curl -s https://turbo.ordinalswallet.com/collection/bitcoin-frogs/inscriptions \
  | python3 -c 'import sys,json; print(json.dumps([x["id"] for x in sorted(json.load(sys.stdin), key=lambda x: x["num"])], indent=0))' \
  | diff - deployments/launch/btc/bitcoin-frogs-members.json && echo same
```

## The further Ordinals collections (listings 12-16, 2026-10-10)

`docs/v9/ordinals-collections.md`. NodeMonkes, Ordinal Maxi Biz (OMB), Bitcoin Puppets, Bitcoin Punks and Natcats, in that order
after Bitcoin Frogs (`nft-pilot.json` `inscriptions[]`). Each `<slug>-members.json` holds the ids two independent sources agree on:
Ordinals Wallet's `turbo /collection/<owSlug>/inscriptions` (the canister's buy venue) and the collection's **on-chain ord gallery
inscription** (`ordGallery`, read from `https://ordinals.com/gallery/<id>`). It is ordered by inscription number, like the Frogs
file. An id only one of them has is left out, never added. `<slug>-members.provenance.json` records both sources and the ids each
alone had. It also compares Magic Eden's legacy export (TheWizardsOfOrd/ordinals-collections `legacy/`) and Ordinals Wallet's 2024
GitHub registry as references, which play no part in the agreement.

| file | members | root | sources (left out) |
|---|---|---|---|
| `nodemonkes-members.json` | 10,000 | `0xe97c65d46ebf87c51e6503705a1ef2df4fbaf5168bd80eecab25d75b10126c09` | OW 10,000, gallery 10,000 |
| `omb-members.json` | 9,000 | `0xb174d1b274c9499b6aba61858c0c6945ca0df0eabf71e269d59527c098dd7af6` | OW 9,001 (1), gallery 9,000 |
| `bitcoin-puppets-members.json` | 10,001 | `0x1aa1697ee4990df1fe27e5bf39d95f2d2b644191eb0cfbf127acf3e92de25d47` | OW 10,001, gallery 10,001 |
| `bitcoin-punks-members.json` | 10,000 | `0xcf39e6081fd4cc6d82c01bec850f06c365a00b5b5c4086494c9e844afe91fda4` | OW 10,000, gallery 10,000 |
| `natcats-members.json` | 8,060 | `0xe5a905f780d8dce46f9be8eac8337752a79a44dc8e8bf090943ee36fe0b925a3` | OW 8,060, gallery 8,064 (4) |

The same generator, run on Bitcoin Frogs (OW + its gallery `659e684f…5a62i0` + Magic Eden's export), gives exactly the committed
10,000 ids and root `0x801f71bb…acfae418e`.

Rebuild and compare (read-only, keyless; about a minute per collection):

```sh
node ops/solana/ordinals-members.mjs fetch nodemonkes /tmp/nm.json --gallery 2ebbd9b93006b69714dd517fbe0d4bf7f8462ffe213105e03048d49ed46eba04i0
diff /tmp/nm.json deployments/launch/btc/nodemonkes-members.json && echo same
node ops/solana/ordinals-members.mjs check      # every Inscriptions entry of nft-pilot.json: count and root recomputed
```

## Listings 17-25 (2026-10-10, operator decision)

The collections the first pass dropped, except Ordinal Punks, appended most liquid first (`docs/v9/ordinals-collections.md` §8). The
same generator and rule; the on-chain source is a gallery (`--gallery`), the collection's parent inscription (`--parent`: its
children) or, for RSIC, a grandparent (`--grandparent`: the children of each of its ten sub-parents).

| id | file | members | root | sources (left out) |
|---|---|---|---|---|
| 17 | `runestone-members.json` | 112,384 | `0x623653a2be69eccaa6666cb091fd6176ced3991df1a98a1bd2bc9e5a89ee7752` | OW 112,384, parent `fdb2df5d…cbb5i0` 112,384 |
| 18 | `bitcoin-weirdos-members.json` | 10,000 | `0xd2a7b78cdb8a5928f63130dc9f05e754d4ea96963c3533d7eb429c75d008d76c` | OW 10,000, gallery 10,000 |
| 19 | `cents-members.json` | 10,000 | `0xa12652ab5011a1d680d7c69414232d1a271a54513173d189103a2afef5ce4849` | OW 10,000, gallery 10,001 (1) |
| 20 | `rsic-members.json` | 20,998 | `0x5c441d676fa812fb48a667eb00759806668b5c0a354fc447e164bf3525d82fee` | OW 20,998, grandparent `aa84eca1…1fb0i0` 21,000 (2) |
| 21 | `pizza-ninjas-members.json` | 2,920 | `0x230aa6d2b7b22d202022427e17d1445a7bf49c6e99da088a3102cd97cc847c55` | OW 2,920, parent `80da3cec…82e1i0` 2,920 |
| 22 | `quantum-cats-members.json` | 3,313 | `0xb4c59b4a21feaf020b68b6efb75e88809f83041651026788b1f952ab03163094` | OW 3,313, parent `0e383b8a…ea41i0` 3,334 (21) |
| 23 | `wizards-of-ord-members.json` | 3,333 | `0xee49af729f84580fd399d60eeeac2bb9eda74a2aa9c3d072760c11695d04a7d9` | OW 3,333, gallery 3,333 |
| 24 | `taproot-wizards-members.json` | 2,102 | `0x6ebe6bf01bd05768e6d2d2c3f6ebf9763a009e2b02513472450b16aba57a9b14` | OW 2,106 (4), gallery 2,102 |
| 25 | `bitcoin-wizards-members.json` | 1,337 | `0x53daf6fc65bea13920360cdd299077ee401229e7d18bddc5d6728a541f56fb8c` | OW 1,337, gallery 1,338 (1) |

`runestone-members.json` is 8.1 MB (112,384 ids), `rsic-members.json` 1.5 MB; every other file is under 0.7 MB. Rebuild (the
Runestone run reads 1,124 pages of `/r/children` and takes about five minutes):

```sh
node ops/solana/ordinals-members.mjs fetch runestone /tmp/rs.json --parent fdb2df5d2b16db1ebcbf09e2d23b3f4e417db44b58e712c99b61f26b52c7cbb5i0
node ops/solana/ordinals-members.mjs fetch rsic /tmp/rsic.json --grandparent aa84eca1e3a967674cbfe7efc72eda3b080918de5a1fe49a4fda00e29fd11fb0i0 --me rsic
node ops/solana/ordinals-members.mjs fetch pizza-ninjas /tmp/pn.json --parent 80da3cecac858f2757c5b338be15980bdc9d9570d4e86765b4948be8143c82e1i0 --me pizza-ninjas
```

The other commands are in each `<slug>-members.provenance.json` (`sources[].url`).
