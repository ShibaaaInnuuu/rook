use anchor_lang::prelude::*;

#[error_code]
pub enum BackingError {
    #[msg("Only the upgrade authority (init) or rook_core's registry owner may do this")]
    Unauthorized,
    #[msg("The account is not owned by Meteora DBC")]
    NotDbcAccount,
    #[msg("The account is not a DBC PoolConfig")]
    NotDbcConfig,
    #[msg("The account is not a DBC VirtualPool (SPL)")]
    NotDbcPool,
    #[msg("The DBC config's fee_claimer is not this collection's claimer PDA")]
    WrongFeeClaimer,
    #[msg("The DBC config's quote mint is not wrapped SOL")]
    WrongQuoteMint,
    #[msg("The DBC config does not migrate to DAMM v2")]
    WrongMigrationOption,
    #[msg("The DBC config's base token is not plain SPL")]
    WrongTokenType,
    #[msg("The DBC config does not collect fees in the quote token")]
    WrongCollectFeeMode,
    #[msg("The DBC config pays no migration fee to the partner")]
    NoMigrationFee,
    #[msg("The DBC config shares the migration fee with the creator")]
    CreatorMigrationFee,
    #[msg("The config is already registered")]
    ConfigAlreadyRegistered,
    #[msg("Too many configs for this collection")]
    TooManyConfigs,
    #[msg("The DBC pool was not created under one of this collection's configs")]
    NotRookPool,
    #[msg("The coin mint is not the pool's base mint")]
    WrongCoinMint,
    #[msg("The DBC pool has not migrated to DAMM v2 yet")]
    NotMigrated,
    #[msg("This coin is already backed")]
    AlreadyBacked,
    #[msg("This coin is not backed yet")]
    NotBacked,
    #[msg("Amount must be greater than zero")]
    ZeroAmount,
    #[msg("The amount redeems zero wX units (too small)")]
    ZeroPayout,
    #[msg("Fewer wX units than the minimum")]
    BelowMinUnits,
    #[msg("The destination is not rook_core's treasury")]
    WrongTreasury,
    #[msg("The position NFT account is not held by the claimer")]
    WrongPositionOwner,
    #[msg("Math overflow")]
    MathOverflow,
    #[msg("The account is not rook_core's Registry")]
    NotRegistry,
    #[msg("The partner's registry is not rook_core's Registry")]
    WrongRegistry,
    #[msg("The DBC config does not use timestamp activation")]
    WrongActivationType,
    #[msg("The DBC pool has no activation time")]
    NoLaunchTime,
    // ---- audit 2026-10 fixes (appended: earlier codes unchanged) ----
    #[msg("The DBC config is not of the version whose layout rook_backing reads (DBC 0.2.1)")]
    WrongConfigVersion,
    #[msg("The DBC config leaves LP unlocked or vesting: partner and creator LP must be permanently locked")]
    UnlockedLiquidity,
    #[msg("The migrated DAMM v2 pool does not collect its fees in SOL only")]
    WrongMigratedFeeMode,
    #[msg("The DBC config charges a pool creation fee")]
    PoolCreationFee,
    #[msg("The DBC config's leftover receiver is not this collection's claimer PDA")]
    WrongLeftoverReceiver,
    #[msg("The account is not this listing's rook_core account")]
    WrongCoreAccount,
    #[msg("The rook_core module has no accepted floor yet")]
    NoFloor,
}
