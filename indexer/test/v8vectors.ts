/**
 * Real drand evmnet beacons, for the v8 tests and fixtures (docs/v8/redemption-draw.md §2.3). CONTRACT: test/DrandEvmnet.t.sol's vectors
 * (the Solidity verifier accepts them). RELAYS: rounds fetched on 2026-09-27 from api.drand.sh and api2.drand.sh, which served
 * identical bytes; `burns` are the fixture's burn timestamps, each pinning roundAt(burn) + 20, the rounds below in order.
 */
export const CONTRACT_VECTORS = [
  { round: 1000, signature: "0x06fd5996329504d3a56b482d9222bf7205857d0a9559ddd216ca31a286f6a8cc0a120f021aac2f13553fb164f62bc3a5ca32c76dea88a777b39bcf3cac5fdbd6", randomness: "0x0e6745667465a6f9dce5d5f994656955080be14c469ff17fc4fc588c925a8504" },
  { round: 20823703, signature: "0x205ff8f2b2cb01f39deb17fe64eed496207c8fa1a490e5bf0db027ced4665bcf22504bf545a9880b36617734cede20622276674ec0c5599561ac68e8f5432ad6", randomness: "0x13b91b6a08f04051999beeca41f8ce3f99e1bf8004a128ba49e3fa27f0c4ba25" },
] as const;
export const FIXTURE_BURNS = [1790100000, 1790150037, 1790200074, 1790250111, 1790300148, 1790350185, 1790400222, 1790450259] as const;
export const RELAY_ROUNDS: Record<number, { signature: `0x${string}`; randomness: `0x${string}` }> = {
  20859662: { signature: "0x011dc6861bbc09918e9c4447058ddcd600e7016903aa021d10c5e531f4a39e5925927891286b5cf2a19098baa1c4d5ad2ebe2272b911e1e8c0566a0c18398a40", randomness: "0x2b4fcca4d066838fefbfb9e3341a3cebc6b0ae98b1ffacf2d82646d3cddc0d83" },
  20876341: { signature: "0x24a5c7f9909c6703e16c2870a03480a9163c843a62c89e575b0a86dacfcab0c501d0276bfbe40a8dc97f19003cefe3b3c1fbb89e26dd8cfb6431abb06788c1dc", randomness: "0x4535f901801a8fc96ba6eddb6fd35dfe196051ed87780120c3317df7e4cbcd95" },
  20893020: { signature: "0x0e903e6c48e3a9b0505aaa4ac429234c060cb710fcfcfb82b5d6505946b3f5f11ef2088fe27551386e6806420fa908da2922556bd6951e372e29a902d4693d96", randomness: "0x8da047a13873ab039d094cdabc3ef063898e7e58b64f7776fac047dc013afd93" },
  20909699: { signature: "0x16d29d362efffc715503587485d494afbf3fc6a5a78c7051091660077798f1cc117ba04b2ea8f646d0da7c38f84f74624e8987daa709f531efd8e97f368ad22a", randomness: "0xaf85d1621d84d44b33e0b9e4b47bb32d1f79931ab16b3a7e2f16d8df51985459" },
  20926378: { signature: "0x1e77f5f3b4eeefb77703440440505fbcb48e23913e2492617d20ace207c868752f9887e414175007721d0466f7ad6d8b2231c7f29dbbb0e4a55c0701b6c20dda", randomness: "0x3c295411f3fa2af0261f5f452d3a965a4addb5bde35e269d334102b227a8cde2" },
  20943057: { signature: "0x143506f534d36aaef66125e055a62f8576f5dac1a6d1ded054deaf243146dd2a2ea0504c8eaa7ba2594a157b74065bed792884c0e1c3e4567cabb5f34f5de43f", randomness: "0x5254735dd1461b4f631d7ec12264924b805f7ca7ad5a6795137ec2a98c2db18a" },
  20959736: { signature: "0x00df97769529307cad908b36b7e7f98da4c7700bd1af1a8a83be7c1f30b98e612935e1a0056fb312e20d8808add29933e2747012cfb0fca0034509be126feb0c", randomness: "0x4919c006a1f4c57ea68502f55b5ba363f02b9d9344164a67c9bc65eede3c4388" },
  20976415: { signature: "0x06c811ddded45f6e571f50ad551b68c3f9e05b41273ee8a6bff0dfcbaff6511c0c8942f4fffb8c417b45c0516167afeb6263ded99a9be8ca0eebad29dfb35773", randomness: "0x7ecd8db7bc9eb0871282bc29f25575134e047be41fde290b5a51d32972964e5d" },
};
