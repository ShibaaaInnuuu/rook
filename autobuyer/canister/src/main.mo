// etched v2 spike (docs/v2/autonomous-buyer.md). LOCAL REPLICA ONLY: the key is dfx's public test key, worthless by construction.
// It proves three capabilities and nothing else: a threshold BIP340 key with the BIP341 tweak, an HTTPS GET, a JSON-RPC POST.
import Blob "mo:base/Blob";
import Cycles "mo:base/ExperimentalCycles";
import Text "mo:base/Text";
import Nat64 "mo:base/Nat64";

persistent actor AutobuyerSpike {
  type SchnorrKeyId = { algorithm : { #bip340secp256k1; #ed25519 }; name : Text };
  type HttpHeader = { name : Text; value : Text };
  type HttpResponse = { status : Nat; headers : [HttpHeader]; body : Blob };
  type TransformArgs = { response : HttpResponse; context : Blob };
  type Management = actor {
    schnorr_public_key : ({ canister_id : ?Principal; derivation_path : [Blob]; key_id : SchnorrKeyId }) -> async ({ public_key : Blob; chain_code : Blob });
    sign_with_schnorr : ({ message : Blob; derivation_path : [Blob]; key_id : SchnorrKeyId; aux : ?{ #bip341 : { merkle_root_hash : Blob } } }) -> async ({ signature : Blob });
    http_request : ({ url : Text; max_response_bytes : ?Nat64; method : { #get; #head; #post }; headers : [HttpHeader]; body : ?Blob;
      transform : ?{ function : shared query TransformArgs -> async HttpResponse; context : Blob }; is_replicated : ?Bool }) -> async HttpResponse;
  };
  transient let ic : Management = actor "aaaaa-aa";
  // On the LOCAL replica (dfx 0.32) the only BIP340 key is a throwaway generated at start and named "key_1". It shares a NAME
  // with mainnet's production key and nothing else: this canister is never deployed anywhere but 127.0.0.1 by this spike.
  transient let KEY : SchnorrKeyId = { algorithm = #bip340secp256k1; name = "key_1" };
  transient let PATH : [Blob] = [Text.encodeUtf8("etched/spike/taproot")];

  /// The canister's own BIP340 public key (33-byte SEC1; the x-only key is its last 32 bytes).
  public func taprootInternalKey() : async Blob {
    (await ic.schnorr_public_key({ canister_id = null; derivation_path = PATH; key_id = KEY })).public_key
  };

  /// Sign a 32-byte BIP341 sighash for a KEY-PATH spend: the subnet applies the taproot tweak itself (`aux = #bip341`), so the
  /// signature verifies under the OUTPUT key Q = P + H_tapTweak(P || merkle_root)·G. An empty root = no script tree.
  public func signKeyPath(sighash : Blob, merkleRoot : Blob) : async Blob {
    assert sighash.size() == 32;
    Cycles.add<system>(30_000_000_000);
    (await ic.sign_with_schnorr({ message = sighash; derivation_path = PATH; key_id = KEY; aux = ?#bip341({ merkle_root_hash = merkleRoot }) })).signature
  };

  /// Strips everything but status and body so replicas can agree (headers carry dates and ids).
  public query func clean(args : TransformArgs) : async HttpResponse {
    { status = args.response.status; headers = []; body = args.response.body }
  };

  /// A read of a public API. `replicated = false` is the mode the design needs for a marketplace's PSBT calls.
  public func get(url : Text, replicated : Bool) : async { status : Nat; body : Text } {
    Cycles.add<system>(60_000_000_000);
    let r = await ic.http_request({ url; max_response_bytes = ?Nat64.fromNat(200_000); method = #get;
      headers = [{ name = "accept"; value = "application/json" }, { name = "user-agent"; value = "etched-spike" }];
      body = null; transform = ?{ function = clean; context = Blob.fromArray([]) }; is_replicated = ?replicated });
    { status = r.status; body = switch (Text.decodeUtf8(r.body)) { case (?t) t; case null "<binary>" } }
  };

  /// One JSON-RPC call to an EVM endpoint (the spike reads Robinhood Chain; it never sends).
  public func rpc(url : Text, json : Text) : async { status : Nat; body : Text } {
    Cycles.add<system>(60_000_000_000);
    let r = await ic.http_request({ url; max_response_bytes = ?Nat64.fromNat(500_000); method = #post;
      headers = [{ name = "content-type"; value = "application/json" }]; body = ?Text.encodeUtf8(json);
      transform = ?{ function = clean; context = Blob.fromArray([]) }; is_replicated = ?true });
    { status = r.status; body = switch (Text.decodeUtf8(r.body)) { case (?t) t; case null "<binary>" } }
  };
};
