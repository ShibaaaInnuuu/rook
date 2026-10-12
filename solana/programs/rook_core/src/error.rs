use anchor_lang::prelude::*;

/// Every refusal of `rook_core`. Names follow the Solidity errors they port (FloorModule, EthNftWrapper/Wrapper,
/// ListingRegistry, PosterSource, ProtocolTreasury, WakeDesk) so a reviewer can map a test to its source.
#[error_code]
pub enum RookError {
    // ---- common ----
    #[msg("arithmetic overflow")]
    MathOverflow,
    #[msg("bad input")]
    BadInput,
    #[msg("bad parameters")]
    BadParams,
    #[msg("not the owner")]
    NotOwner,
    #[msg("not the owner or the guardian")]
    NotGuardian,
    #[msg("not the keeper")]
    NotKeeper,
    #[msg("not the custodian")]
    NotCustodian,
    #[msg("not an allowed converter")]
    NotConverter,
    #[msg("the same key may not hold two roles")]
    SameKey,
    #[msg("nothing pending")]
    NothingPending,
    #[msg("too early")]
    TooEarly,
    #[msg("too late")]
    TooLate,
    #[msg("wrong listing kind")]
    WrongKind,
    #[msg("empty")]
    Empty,
    #[msg("list full")]
    Full,
    #[msg("account does not belong to this collection")]
    WrongCollection,
    // ---- floor ----
    #[msg("the floor is stale")]
    Stale,
    #[msg("the keeper (or this collection's poster) is paused")]
    Paused,
    #[msg("slippage")]
    Slippage,
    #[msg("over the hourly cap")]
    OverCap,
    #[msg("over the ETH-backed cap")]
    OverBacked,
    #[msg("not enough backed units")]
    NotEnoughBacked,
    #[msg("over the basis: revenue and surplus cannot cover the item cost")]
    OverBasis,
    #[msg("the item cap is stale")]
    CapStale,
    #[msg("over the item cap")]
    OverItemCap,
    #[msg("the module holds no wrapped unit")]
    NoWrapped,
    #[msg("item arrivals are rate limited")]
    ArrivalRateLimited,
    #[msg("an advance is open")]
    AdvanceOpen,
    #[msg("no advance is open")]
    NoAdvance,
    #[msg("nothing to settle")]
    NothingToSettle,
    // ---- wrap ----
    #[msg("the listing is paused")]
    ListingPaused,
    #[msg("the item is already held")]
    AlreadyHeld,
    #[msg("this (item, transaction) already minted")]
    AlreadyMinted,
    #[msg("not in the collection")]
    NotInCollection,
    #[msg("mint rate limited")]
    MintRateLimited,
    #[msg("the wrapper is not fresh")]
    NotFresh,
    #[msg("bad destination")]
    BadDestination,
    #[msg("no unpromised item")]
    NoItem,
    #[msg("wrong stage")]
    WrongStage,
    #[msg("out of order")]
    OutOfOrder,
    #[msg("nothing to draw")]
    NothingToDraw,
    #[msg("not marked undeliverable")]
    NotUndeliverable,
    #[msg("a delivery was noted sent and not recorded")]
    DeliverySentNotRecorded,
    #[msg("the drand round is not proven")]
    RoundNotProven,
    #[msg("custody is full")]
    CustodyFull,
    // ---- treasury ----
    #[msg("the shares must sum to 10000")]
    BadShares,
    #[msg("already a collection")]
    AlreadyCollection,
    #[msg("not a collection")]
    NotCollection,
    #[msg("nothing to do")]
    NothingToDo,
    // ---- wake ----
    #[msg("wrong bond")]
    WrongBond,
    #[msg("a request for this collection is open")]
    AlreadyRequested,
    #[msg("the collection is awake")]
    Awake,
    #[msg("closed")]
    Closed,
    #[msg("not a launch of the requester after the request")]
    NotALaunch,
    #[msg("not graduated")]
    NotGraduated,
    #[msg("this coin already paid back a bond")]
    AlreadySpent,
    #[msg("not served")]
    NotServed,
    #[msg("not closed")]
    NotClosed,
    // ---- registry (appended: earlier codes unchanged) ----
    #[msg("not the program's upgrade authority")]
    NotUpgradeAuthority,
    // ---- audit 2026-10 fixes (appended: earlier codes unchanged) ----
    #[msg("a distribution round is still open (an index with weight is unpaid); it may be restarted a day after it began")]
    RoundOpen,
    #[msg("the drand round is not live yet")]
    RoundInFuture,
    #[msg("the destination is the listing's custody address")]
    CustodyDestination,
    #[msg("the ticket's drand round is proven: resolve it instead of refunding")]
    RoundProven,
    #[msg("the held marker does not point to this item")]
    WrongHeldMarker,
    #[msg("bad token metadata (name 1-32, symbol 1-10, uri at most 200 bytes)")]
    BadMetadata,
    // ---- the reserve cap (appended 2026-10-10: earlier codes unchanged) ----
    #[msg("the reserve cap must be 1000..=200000 bps of the floor")]
    BadReserveCap,
    // ---- the holder raffle (appended 2026-10-10: earlier codes unchanged) ----
    #[msg("the raffle share must be at most 10000 bps")]
    BadRaffleBps,
    #[msg("the raffle already holds a whole wX prize")]
    PrizeHeld,
    #[msg("the raffle pot cannot pay for one whole wX at the current price")]
    PotTooSmall,
    #[msg("the raffle is not in the phase this needs")]
    RafflePhase,
    #[msg("a raffle round may be committed only 7 days after the previous one")]
    RaffleTooEarly,
    #[msg("the raffle holds no whole wX prize")]
    NoPrize,
    #[msg("bad snapshot slot (in the past, at most 2 days old, after the previous one)")]
    BadSnapshot,
    #[msg("the claim window is over")]
    ClaimWindowOver,
    #[msg("the leaf does not contain the winning ticket")]
    NotTheWinner,
    #[msg("bad Merkle proof")]
    BadProof,
    #[msg("the raffle round has not expired")]
    RaffleNotExpired,
    // ---- Solana NFT listings (appended 2026-10-10: earlier codes unchanged; docs/v9/solana-nfts-design.md) ----
    #[msg("not a supported NFT of this listing (standard, mint, metadata or master edition)")]
    NotAnNft,
    #[msg("the asset or its collection carries a plugin that could freeze, move or burn it in custody, or veto its transfer")]
    UnsupportedNft,
    #[msg("the NFT is not held by the expected wallet (the custodian at a mint, the destination at a delivery)")]
    NotInCustody,
    #[msg("an account this standard needs is missing from remaining_accounts")]
    MissingAccount,
}
