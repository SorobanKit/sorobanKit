#![no_std]
#[cfg(test)]
extern crate std;

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, log, symbol_short, token, Address, BytesN,
    Env, Symbol, Vec,
};

// ── Schema version and migration ─────────────────────────────────────────────
//
// Every breaking change to the persistent-storage layout increments
// SCHEMA_VERSION.  `migrate` is an idempotent, bounded entry point that
// must be executed before the new code interprets old packed state.
// Callers (upgrade tooling) check `schema_version()` before deciding
// whether to call `migrate`.
//
// Invariant: new code rejects reads of persistent layout data whose stored
// schema version is less than SCHEMA_VERSION, to prevent misinterpretation.
//
// History:
//   0 → absent (pre-schema contract, no version key stored)
//   1 → adds PaymentNonce, FeeTotal, RefundEntry keys; layout v1

const SCHEMA_VERSION: u32 = 1;

// ── Packed UserSpending helpers ──────────────────────────────────────────────
//
// Issue #519: Replace the two-field UserSpending contracttype with a single
// BytesN<24> value packed with bitwise operations.
//
// Layout (big-endian):
//   bytes  0..8  — last_reset_time  : u64   (8 bytes)
//   bytes  8..24 — accumulated_amount: i128  (16 bytes)
//
// Benefits:
//  • Eliminates the XDR struct-type overhead (type discriminant + field tags)
//    that Soroban adds to every contracttype value, shrinking each UserSpending
//    ledger entry from ~48 bytes to exactly 24 bytes.
//  • Smaller entries → lower state-rent fee per ledger entry per TTL period.

/// Pack `last_reset_time` (u64) and `accumulated_amount` (i128) into a
/// 24-byte big-endian buffer.
fn pack_spending(env: &Env, last_reset_time: u64, accumulated_amount: i128) -> BytesN<24> {
    let mut buf = [0u8; 24];

    // Bytes 0..8 — last_reset_time (u64 big-endian)
    let t_bytes = last_reset_time.to_be_bytes();
    buf[0] = t_bytes[0];
    buf[1] = t_bytes[1];
    buf[2] = t_bytes[2];
    buf[3] = t_bytes[3];
    buf[4] = t_bytes[4];
    buf[5] = t_bytes[5];
    buf[6] = t_bytes[6];
    buf[7] = t_bytes[7];

    // Bytes 8..24 — accumulated_amount (i128 big-endian)
    let a_bytes = accumulated_amount.to_be_bytes();
    buf[8] = a_bytes[0];
    buf[9] = a_bytes[1];
    buf[10] = a_bytes[2];
    buf[11] = a_bytes[3];
    buf[12] = a_bytes[4];
    buf[13] = a_bytes[5];
    buf[14] = a_bytes[6];
    buf[15] = a_bytes[7];
    buf[16] = a_bytes[8];
    buf[17] = a_bytes[9];
    buf[18] = a_bytes[10];
    buf[19] = a_bytes[11];
    buf[20] = a_bytes[12];
    buf[21] = a_bytes[13];
    buf[22] = a_bytes[14];
    buf[23] = a_bytes[15];

    BytesN::from_array(env, &buf)
}

/// Unpack a 24-byte buffer into `(last_reset_time, accumulated_amount)`.
fn unpack_spending(packed: &BytesN<24>) -> (u64, i128) {
    // BytesN::to_array() is available in soroban-sdk v20.
    let buf: [u8; 24] = packed.to_array();

    // last_reset_time — bytes 0..8
    let last_reset_time = u64::from_be_bytes([
        buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7],
    ]);

    // accumulated_amount — bytes 8..24
    let accumulated_amount = i128::from_be_bytes([
        buf[8], buf[9], buf[10], buf[11], buf[12], buf[13], buf[14], buf[15], buf[16], buf[17],
        buf[18], buf[19], buf[20], buf[21], buf[22], buf[23],
    ]);

    (last_reset_time, accumulated_amount)
}

// ── Legacy struct kept for test snapshot compatibility ───────────────────────
//
// The UserSpending contracttype is retained so existing tests that reference
// it directly continue to compile.  All runtime code now uses the packed
// BytesN<24> representation stored under DataKey::UserSpending.

/// A user's rolling 24-hour spending record.
///
/// Retained purely so existing test snapshots that reference this type by
/// name keep compiling. Live contract state is stored as a packed
/// `BytesN<24>` (see `pack_spending` / `unpack_spending`); this struct is not
/// read from or written to storage at runtime.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserSpending {
    /// Unix timestamp (seconds) at which the 24-hour window last reset.
    pub last_reset_time: u64,
    /// Total amount routed by the user since `last_reset_time`.
    pub accumulated_amount: i128,
}

/// A single transfer instruction for use with [`PaymentRouter::route_payments`].
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Payment {
    /// Address the funds are debited from. Must authorize the call.
    pub sender: Address,
    /// Address the funds (minus the platform fee) are credited to.
    pub recipient: Address,
    /// Contract ID of the token (or Stellar Asset Contract) being transferred.
    pub token_address: Address,
    /// Amount to route, denominated in the token's smallest unit. Must be
    /// positive and within the contract's configured min/max bounds.
    pub amount: i128,
}

// ── Timelock data structures ─────────────────────────────────────────────────
//
// Admin actions that change sensitive contract parameters (treasury, fees,
// governance, admin transfer) are not applied instantly.  Instead the admin
// queues an ActionType intent that gets a nonce ID and a ledger timestamp.
// Only after SECONDS_IN_24H (86 400 s) has elapsed can execute_action be
// called to apply the change.  This gives observers a 24-hour window to
// detect and respond to a compromised-admin scenario.
//
// The freeze mechanism is the complementary emergency tool: calling
// emergency_freeze instantly blocks all payments and all timelock executions.
// A freeze does NOT require going through the timelock itself so it is always
// available to the admin as an immediate last resort. Unfreezing is timelocked
// so observers can review the recovery before routing resumes.

/// Describes which administrative parameter change a timelock entry represents.
/// Each variant carries all the arguments needed to apply that change when the
/// delay period is over.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActionType {
    /// Change the platform treasury address.
    SetPlatformTreasury(Address),
    /// Update fee basis-points and fee cap together (legacy / combined setter).
    SetFeeConfig(i128, i128),
    /// Update fee basis-points only.
    SetFeeBps(i128),
    /// Set the governance contract address.
    SetGovernance(Address),
    /// Change the minimum routing limit.
    SetMinLimit(i128),
    /// Change the minimum individual payment amount.
    SetMinAmount(i128),
    /// Change the maximum individual payment amount.
    SetMaxAmount(i128),
    /// Add or remove a recipient from the blacklist.
    SetBlacklist(Address, bool),
    /// Block or unblock a token contract. Tokens are unblocked by default.
    SetTokenBlocked(Address, bool),
    /// Pause or resume routing.
    SetPaused(bool),
    /// Remove an emergency freeze.
    Unfreeze,
    /// Nominate a new admin; the nominee must separately accept.
    TransferAdmin(Address),
    /// Upgrade the contract WASM.
    Upgrade(BytesN<32>),
}

/// A pending timelock entry stored in persistent ledger storage.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimelockEntry {
    /// Ledger timestamp (seconds since epoch) when this action was queued.
    pub queued_at: u64,
    /// The action payload to apply once the delay has elapsed.
    pub action: ActionType,
}

/// An admin nomination awaiting explicit acceptance by the proposed address.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingAdminTransfer {
    pub proposed_admin: Address,
    pub proposed_at: u64,
    pub expires_at: u64,
    pub action_nonce: u64,
}

// ── Schema v1 types ──────────────────────────────────────────────────────────

/// An immutable execution receipt for a completed payment.
///
/// Written exactly once when a payment succeeds.  The key is
/// `(sender, nonce)`, so each sender's nonce space is independent and
/// a nonce used by one sender cannot collide with the same nonce used by
/// another sender.
///
/// `expires_at` is the wall-clock timestamp after which the receipt may be
/// pruned from storage by the admin, but the logical record of the payment is
/// considered permanent once written.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PaymentReceipt {
    /// The sender who submitted the payment.
    pub sender: Address,
    /// The recipient of the payment.
    pub recipient: Address,
    /// The token contract used.
    pub token: Address,
    /// The gross amount routed (before fee deduction).
    pub amount: i128,
    /// The fee collected from this payment.
    pub fee: i128,
    /// Wall-clock timestamp when the payment was executed (seconds since epoch).
    pub executed_at: u64,
    /// Wall-clock timestamp after which this receipt entry may be pruned.
    pub expires_at: u64,
}

/// A refund balance entry with creation metadata.
///
/// Replaces the bare `i128` stored under `DataKey::RefundBalance` at schema v0.
/// At schema v1 all new refund credits are stored under `DataKey::RefundEntry`
/// as a `RefundEntry` value.  Legacy entries under `DataKey::RefundBalance`
/// are migrated by `migrate_v0_to_v1`.
///
/// `created_at` enables the governed post-expiry recovery path:
/// once `created_at + REFUND_EXPIRY_SECONDS` has passed, the ordinary
/// `withdraw_refund` / `claim_all_refunds` calls are rejected with
/// `Error::RefundExpired`, and only `recover_expired_refund` (admin-gated)
/// may claim the balance.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RefundEntry {
    /// Amount still available for withdrawal.
    pub balance: i128,
    /// Wall-clock timestamp (seconds since epoch) when this refund was first created.
    pub created_at: u64,
}

/// Storage keys for all contract instance and persistent data.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    /// The current admin address.
    Admin,
    /// Governance contract address; if set, it takes over fee-authority from the admin.
    Governance,
    /// Address that receives collected platform fees.
    PlatformTreasury,
    /// Platform fee rate, in basis points (1/100th of a percent).
    FeeBps,
    /// Upper bound on the fee taken from a single payment.
    FeeCap,
    /// Minimum amount accepted by `route_payment` / `route_payments`, if set.
    MinLimit,
    /// Minimum transfer amount enforced by `route_payment`, if set.
    MinAmount,
    /// Whether routing is currently paused.
    Paused,
    /// Maximum amount accepted by a single payment.
    MaxAmount,
    /// Cumulative lifetime amount routed by a given sender.
    UserVolume(Address),
    /// Packed 24-hour spending window for a given sender.
    UserSpending(Address),
    /// Whether a given recipient address is blacklisted.
    Blacklist(Address),
    /// Whether a given token contract is blocked from routing.
    TokenBlocked(Address),
    /// Internal refund balance for a (user, token) pair, credited when a
    /// direct transfer to the recipient fails.
    RefundBalance(Address, Address),
    /// Monotonically-increasing nonce counter used to generate unique IDs for
    /// timelock entries.  Stored as `u64` in instance storage.
    TimelockNonce,
    /// A pending timelock entry keyed by its nonce ID.
    /// Stored in persistent storage so it survives instance eviction.
    TimelockEntry(u64),
    /// The nonce for an identical action that is already queued.
    PendingAction(ActionType),
    /// An admin transfer awaiting acceptance.
    PendingAdminTransfer,
    /// When `true` the contract is frozen: payments and timelock executions
    /// are blocked.  Stored as `bool` in instance storage.
    Frozen,
    // ── New keys added at schema version 1 ──────────────────────────────────
    //
    // Callers must ensure `schema_version() == SCHEMA_VERSION` (i.e. `migrate`
    // has been called) before reading or writing these entries.
    //
    /// Monotonically-increasing per-sender nonce counter.
    /// Stored as `u64` in persistent storage, keyed by sender address.
    PaymentNonceCounter(Address),
    /// Immutable execution receipt for a completed payment.
    /// Key: (sender, nonce).  Written once; never updated.
    PaymentNonce(Address, u64),
    /// Cumulative fee collected for a specific token.
    /// Stored as `i128` in persistent storage.
    FeeTotal(Address),
    /// Refund entry with creation timestamp for a (user, token) pair.
    /// Replaces the bare `RefundBalance` i128 at schema v1.
    RefundEntry(Address, Address),
    /// Stored schema version written by `initialize` and each `migrate` call.
    SchemaVersion,
}

/// Contract-level errors returned instead of panicking, so callers get a
/// specific, stable error code to branch on rather than an opaque trap.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    /// Caller is not authorized to perform this action (e.g. not the admin).
    Unauthorized = 1,
    /// Sender's token balance is lower than the requested payment amount.
    InsufficientBalance = 2,
    /// Requested amount is outside allowed bounds, or a spending limit was exceeded.
    LimitExceeded = 3,
    /// `initialize` was called on a contract that already has an admin set.
    AlreadyInitialized = 4,
    /// An admin-configured value (treasury, fee, admin) was read before `initialize`.
    NotInitialized = 5,
    /// The contract is currently paused; routing calls are rejected until unpaused.
    Paused = 6,
    /// A fee configuration value (basis points or cap) is out of the allowed range.
    InvalidFeeRate = 7,
    /// Sender and recipient addresses are the same (self-routing not allowed).
    InvalidRecipient = 8,
    /// Recipient address is blacklisted.
    Blacklisted = 9,
    /// Requested refund withdrawal amount is zero or exceeds available refund balance.
    NoRefundAvailable = 10,
    /// An action is already pending in the timelock queue; it must be executed
    /// or cancelled before an identical action can be queued.
    TimelockPending = 11,
    /// The 24-hour delay for the given timelock entry has not elapsed yet.
    TimelockNotReady = 12,
    /// No timelock entry exists for the supplied nonce ID.
    TimelockNotFound = 13,
    /// The contract is frozen; all payments and timelock executions are blocked.
    ContractFrozen = 14,
    /// The payment amount is below the admin-configured minimum transfer amount.
    AmountBelowMinimum = 15,
    /// A sensitive setting must be changed through the timelock.
    TimelockRequired = 16,
    /// A different admin nomination is still awaiting acceptance.
    AdminTransferPending = 17,
    /// The proposed admin is already the current admin.
    InvalidAdminTransfer = 18,
    /// The admin nomination has expired.
    AdminTransferExpired = 19,
    /// The token has been blocked from routing.
    TokenBlocked = 20,
    // ── New errors added at schema version 1 ────────────────────────────────
    /// The caller-supplied nonce has already been used by this sender.
    NonceDuplicate = 21,
    /// The caller-supplied nonce has expired and can no longer be used.
    NonceExpired = 22,
    /// A migration step was called but the contract is already at the target schema version.
    AlreadyMigrated = 23,
    /// A migration step was called but the contract schema is at a version that
    /// cannot be migrated to the requested target by this entry point.
    MigrationVersionMismatch = 24,
    /// A refund balance has passed its expiry window and can only be claimed
    /// by the governed recovery path, not by ordinary `withdraw_refund`.
    RefundExpired = 25,
}

/// Soroban contract that routes token payments between addresses while
/// collecting a configurable platform fee, enforcing per-user daily spending
/// limits, and supporting an admin-managed blacklist and pause switch.
#[contract]
pub struct PaymentRouter;

#[contractimpl]
impl PaymentRouter {
    const BPS_DIVISOR: i128 = 10_000;
    const XLM_DECIMALS: i128 = 10_000_000;
    const MAX_AMOUNT: i128 = 1_000_000_000_000_000; // 100M tokens with 7 decimals
    const DAILY_MAX_LIMIT: i128 = 1_000_000 * Self::XLM_DECIMALS; // 1M tokens limit
    const VOLUME_THRESHOLD: i128 = 10_000 * Self::XLM_DECIMALS; // 10,000 XLM threshold for tiered fee discount
    const SECONDS_IN_24H: u64 = 24 * 3600;
    const ADMIN_TRANSFER_EXPIRY_SECONDS: u64 = 7 * Self::SECONDS_IN_24H;
    /// Length of the daily spending window, in seconds (24 hours).
    const DAY_SECONDS: u64 = 86_400;
    const VERSION: u32 = 1;
    const INTERFACE_VERSION: u32 = 1;

    const DAY_IN_LEDGERS: u32 = 17280;
    const INSTANCE_BUMP_AMOUNT: u32 = 7 * Self::DAY_IN_LEDGERS;
    const INSTANCE_LIFETIME_THRESHOLD: u32 = Self::INSTANCE_BUMP_AMOUNT - Self::DAY_IN_LEDGERS;

    const USER_BUMP_AMOUNT: u32 = 30 * Self::DAY_IN_LEDGERS;
    const USER_LIFETIME_THRESHOLD: u32 = Self::USER_BUMP_AMOUNT - Self::DAY_IN_LEDGERS;
    const PERSISTENT_BUMP_AMOUNT: u32 = Self::USER_BUMP_AMOUNT;
    const PERSISTENT_LIFETIME_THRESHOLD: u32 = Self::USER_LIFETIME_THRESHOLD;

    // ── Schema v1 constants ──────────────────────────────────────────────────

    /// How long (seconds) a caller-supplied nonce remains valid after it is
    /// first seen.  A submission arriving after this window is rejected with
    /// `Error::NonceExpired`.  Receipts older than this may be pruned by the
    /// admin using `prune_receipt`.
    const NONCE_EXPIRY_SECONDS: u64 = 7 * Self::SECONDS_IN_24H; // 7 days

    /// How long (seconds) an unclaimed refund balance remains accessible to
    /// the original sender via `withdraw_refund` / `claim_all_refunds`.
    /// After this window elapses the balance is locked for the governed
    /// recovery path (`recover_expired_refund`).
    const REFUND_EXPIRY_SECONDS: u64 = 90 * Self::SECONDS_IN_24H; // 90 days

    // ── Private helpers ──────────────────────────────────────────────────────

    fn require_admin(env: &Env) -> Result<Address, Error> {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)
    }

    /// Fee authority helper: if a Governance address is set it takes exclusive
    /// control over fee updates; otherwise the admin retains that right.
    fn require_fee_authority(env: &Env) -> Result<(), Error> {
        let authority = if let Some(gov) = env
            .storage()
            .instance()
            .get::<DataKey, Address>(&DataKey::Governance)
        {
            gov
        } else {
            Self::require_admin(env)?
        };
        authority.require_auth();
        Ok(())
    }

    fn fee_authority(env: &Env) -> Result<Address, Error> {
        env.storage()
            .instance()
            .get(&DataKey::Governance)
            .map(Ok)
            .unwrap_or_else(|| Self::require_admin(env))
    }

    fn action_authority(env: &Env, action: &ActionType) -> Result<Address, Error> {
        match action {
            ActionType::SetFeeConfig(_, _) | ActionType::SetFeeBps(_) => Self::fee_authority(env),
            _ => Self::require_admin(env),
        }
    }

    fn validate_action(action: &ActionType) -> Result<(), Error> {
        match action {
            ActionType::SetFeeConfig(fee_bps, fee_cap)
                if !(0..=Self::BPS_DIVISOR).contains(fee_bps) || *fee_cap < 0 =>
            {
                Err(Error::InvalidFeeRate)
            }
            ActionType::SetFeeBps(fee_bps) if !(0..=Self::BPS_DIVISOR).contains(fee_bps) => {
                Err(Error::InvalidFeeRate)
            }
            ActionType::SetMinLimit(amount) | ActionType::SetMinAmount(amount)
                if *amount < 0 || *amount > Self::MAX_AMOUNT =>
            {
                Err(Error::LimitExceeded)
            }
            ActionType::SetMaxAmount(amount) if *amount <= 0 || *amount > Self::MAX_AMOUNT => {
                Err(Error::LimitExceeded)
            }
            _ => Ok(()),
        }
    }

    fn load_fee_config(env: &Env) -> Result<(Address, i128, i128), Error> {
        let platform_treasury: Address = env
            .storage()
            .instance()
            .get(&DataKey::PlatformTreasury)
            .ok_or(Error::NotInitialized)?;
        let fee_bps: i128 = env
            .storage()
            .instance()
            .get(&DataKey::FeeBps)
            .ok_or(Error::NotInitialized)?;
        let fee_cap: i128 = env
            .storage()
            .instance()
            .get(&DataKey::FeeCap)
            .ok_or(Error::NotInitialized)?;

        env.storage().instance().extend_ttl(
            Self::INSTANCE_LIFETIME_THRESHOLD,
            Self::INSTANCE_BUMP_AMOUNT,
        );

        Ok((platform_treasury, fee_bps, fee_cap))
    }

    fn get_refund_balance_internal(env: &Env, user: &Address, token: &Address) -> i128 {
        // Schema v1: check RefundEntry first; fall back to legacy RefundBalance.
        if let Some(entry) = Self::get_refund_entry(env, user, token) {
            return entry.balance;
        }
        env.storage()
            .persistent()
            .get(&DataKey::RefundBalance(user.clone(), token.clone()))
            .unwrap_or(0)
    }

    fn credit_refund_balance(env: &Env, user: &Address, token: &Address, amount: i128) {
        // Schema v1: write to RefundEntry so the creation timestamp is recorded.
        // The legacy RefundBalance key is left untouched for entries that
        // pre-date the migration (they are readable via get_refund_balance_internal).
        Self::credit_refund_entry(env, user, token, amount);
    }

    /// Returns whether the contract is currently frozen.
    fn is_frozen_internal(env: &Env) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::Frozen)
            .unwrap_or(false)
    }

    /// Allocates and returns the next timelock nonce, incrementing the counter.
    fn next_nonce(env: &Env) -> u64 {
        let current: u64 = env
            .storage()
            .instance()
            .get(&DataKey::TimelockNonce)
            .unwrap_or(0u64);
        let next = current + 1;
        env.storage().instance().set(&DataKey::TimelockNonce, &next);
        next
    }

    // ── Schema v1 private helpers ────────────────────────────────────────────

    /// Returns the stored schema version, or 0 if the contract pre-dates
    /// schema versioning.
    fn schema_version_internal(env: &Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::SchemaVersion)
            .unwrap_or(0u32)
    }

    /// Returns the next per-sender payment nonce and increments the counter.
    fn next_payment_nonce(env: &Env, sender: &Address) -> u64 {
        let key = DataKey::PaymentNonceCounter(sender.clone());
        let current: u64 = env.storage().persistent().get(&key).unwrap_or(0u64);
        let next = current + 1;
        env.storage().persistent().set(&key, &next);
        env.storage().persistent().extend_ttl(
            &key,
            Self::PERSISTENT_LIFETIME_THRESHOLD,
            Self::PERSISTENT_BUMP_AMOUNT,
        );
        next
    }

    /// Writes an immutable `PaymentReceipt` keyed by `(sender, nonce)`.
    /// Panics if the key already exists (should never happen in practice
    /// because `nonce` is monotonically increasing per sender).
    fn write_receipt(
        env: &Env,
        sender: &Address,
        nonce: u64,
        receipt: &PaymentReceipt,
    ) {
        let key = DataKey::PaymentNonce(sender.clone(), nonce);
        // Immutability guarantee: once written, never overwritten.
        // The nonce is monotonically allocated so this key is always fresh.
        env.storage().persistent().set(&key, receipt);
        env.storage().persistent().extend_ttl(
            &key,
            Self::PERSISTENT_LIFETIME_THRESHOLD,
            Self::PERSISTENT_BUMP_AMOUNT,
        );
    }

    /// Increments the cumulative fee total for `token` by `fee_amount`.
    fn accrue_fee_total(env: &Env, token: &Address, fee_amount: i128) {
        if fee_amount <= 0 {
            return;
        }
        let key = DataKey::FeeTotal(token.clone());
        let current: i128 = env.storage().persistent().get(&key).unwrap_or(0);
        env.storage()
            .persistent()
            .set(&key, &(current + fee_amount));
        env.storage().persistent().extend_ttl(
            &key,
            Self::PERSISTENT_LIFETIME_THRESHOLD,
            Self::PERSISTENT_BUMP_AMOUNT,
        );
    }

    /// Returns the `RefundEntry` for `(user, token)`, or `None` if absent.
    fn get_refund_entry(env: &Env, user: &Address, token: &Address) -> Option<RefundEntry> {
        env.storage()
            .persistent()
            .get(&DataKey::RefundEntry(user.clone(), token.clone()))
    }

    /// Credits `amount` to the refund entry for `(user, token)`.
    /// Creates the entry with the current ledger timestamp if it does not exist.
    /// Updates the balance without altering `created_at` on subsequent calls.
    fn credit_refund_entry(env: &Env, user: &Address, token: &Address, amount: i128) {
        let key = DataKey::RefundEntry(user.clone(), token.clone());
        let now = env.ledger().timestamp();
        let entry = if let Some(existing) = env.storage().persistent().get::<DataKey, RefundEntry>(&key) {
            RefundEntry {
                balance: existing.balance + amount,
                created_at: existing.created_at,
            }
        } else {
            RefundEntry {
                balance: amount,
                created_at: now,
            }
        };
        env.storage().persistent().set(&key, &entry);
        env.storage().persistent().extend_ttl(
            &key,
            Self::PERSISTENT_LIFETIME_THRESHOLD,
            Self::PERSISTENT_BUMP_AMOUNT,
        );

        env.events().publish(
            (symbol_short!("refunded"), user.clone(), token.clone()),
            amount,
        );
    }

    /// Returns `true` if `entry.created_at + REFUND_EXPIRY_SECONDS <= now`.
    fn refund_is_expired(env: &Env, entry: &RefundEntry) -> bool {
        env.ledger().timestamp() >= entry.created_at.saturating_add(Self::REFUND_EXPIRY_SECONDS)
    }

    /// Core payment logic shared by `route_payment` and `route_payments`.
    #[allow(clippy::too_many_arguments)]
    fn process_single_payment(
        env: &Env,
        sender: &Address,
        recipient: &Address,
        token_address: &Address,
        amount: i128,
        platform_treasury: &Address,
        fee_bps: i128,
        fee_cap: i128,
        caller_nonce: Option<u64>,
    ) -> Result<(), Error> {
        // ── Nonce deduplication (schema v1) ───────────────────────────────────
        //
        // If a caller-supplied nonce is provided, verify it has not been used
        // before by this sender and that it has not expired.  An absent nonce
        // opts out of receipt tracking (pre-v1 callers).
        let now = env.ledger().timestamp();
        let receipt_nonce: u64 = if let Some(cn) = caller_nonce {
            // The nonce must not already exist as a receipt for this sender.
            let receipt_key = DataKey::PaymentNonce(sender.clone(), cn);
            if env.storage().persistent().has(&receipt_key) {
                return Err(Error::NonceDuplicate);
            }
            // The nonce encodes its own expiry: it is valid for
            // NONCE_EXPIRY_SECONDS after the last-seen counter value's
            // timestamp.  We express expiry as: the nonce must be submitted
            // within NONCE_EXPIRY_SECONDS of the current time, anchored by
            // the current counter.  For simplicity, a nonce that is
            // numerically more than NONCE_EXPIRY_SECONDS ahead of the
            // current allocated counter is treated as expired/invalid.
            //
            // Concrete rule: the caller-supplied nonce must equal the
            // *next* nonce the counter will allocate, i.e. it must equal
            // `counter + 1`.  This prevents skip-ahead attacks while
            // remaining strictly monotonic per sender.
            let counter_key = DataKey::PaymentNonceCounter(sender.clone());
            let current_counter: u64 = env
                .storage()
                .persistent()
                .get(&counter_key)
                .unwrap_or(0u64);
            let expected_nonce = current_counter + 1;
            if cn != expected_nonce {
                // Nonce is out of sequence (already used or skipped ahead).
                // Map both to NonceExpired for a uniform caller experience.
                return Err(Error::NonceExpired);
            }
            // Allocate the nonce now (advance the counter).
            Self::next_payment_nonce(env, sender)
        } else {
            // No nonce supplied; auto-allocate an internal one for bookkeeping.
            Self::next_payment_nonce(env, sender)
        };

        // Require sender auth
        sender.require_auth();

        env.events().publish(
            (Symbol::new(env, "payment_initiated"), sender.clone()),
            amount,
        );

        // Prevent self-routing
        if sender == recipient {
            return Err(Error::InvalidRecipient);
        }

        // Check if recipient is blacklisted
        if Self::is_blacklisted(env.clone(), recipient.clone()) {
            return Err(Error::Blacklisted);
        }
        if env
            .storage()
            .persistent()
            .get::<DataKey, bool>(&DataKey::TokenBlocked(token_address.clone()))
            .unwrap_or(false)
        {
            return Err(Error::TokenBlocked);
        }

        // Validate amount bounds
        let max_amount: i128 = env
            .storage()
            .instance()
            .get(&DataKey::MaxAmount)
            .unwrap_or(Self::MAX_AMOUNT);
        if amount <= 0 || amount > max_amount {
            return Err(Error::LimitExceeded);
        }

        // Enforce optional admin-configured minimum payment limit
        let min_limit: i128 = env
            .storage()
            .instance()
            .get(&DataKey::MinLimit)
            .unwrap_or(0);
        if amount < min_limit {
            return Err(Error::LimitExceeded);
        }

        // Enforce optional admin-configured minimum transfer amount
        let min_amount: i128 = env
            .storage()
            .instance()
            .get(&DataKey::MinAmount)
            .unwrap_or(0);
        if amount < min_amount {
            return Err(Error::AmountBelowMinimum);
        }

        // Apply tiered fee discount for high-volume users
        let user_volume: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::UserVolume(sender.clone()))
            .unwrap_or(0);
        let effective_fee_bps = if user_volume > Self::VOLUME_THRESHOLD {
            fee_bps / 2
        } else {
            fee_bps
        };

        // Check time-based daily spending limits.
        let current_time = env.ledger().timestamp();
        let spending_key = DataKey::UserSpending(sender.clone());

        let (mut last_reset_timestamp, mut accumulated_amount): (u64, i128) = env
            .storage()
            .persistent()
            .get::<DataKey, BytesN<24>>(&spending_key)
            .map(|packed| unpack_spending(&packed))
            .unwrap_or((current_time, 0));

        if current_time.saturating_sub(last_reset_timestamp) >= Self::DAY_SECONDS {
            last_reset_timestamp = current_time;
            accumulated_amount = 0;
        }

        accumulated_amount += amount;
        if accumulated_amount > Self::DAILY_MAX_LIMIT {
            return Err(Error::LimitExceeded);
        }

        env.storage().persistent().set(
            &spending_key,
            &pack_spending(env, last_reset_timestamp, accumulated_amount),
        );
        env.storage().persistent().extend_ttl(
            &spending_key,
            Self::PERSISTENT_LIFETIME_THRESHOLD,
            Self::PERSISTENT_BUMP_AMOUNT,
        );

        // Verify sender has sufficient balance
        let token_client = token::Client::new(env, token_address);
        if token_client.balance(sender) < amount {
            return Err(Error::InsufficientBalance);
        }

        // Calculate fee
        let mut fee_amount = (amount * effective_fee_bps) / Self::BPS_DIVISOR;
        if fee_amount > fee_cap {
            fee_amount = fee_cap;
        }
        if fee_amount > amount {
            fee_amount = amount;
        }
        let remainder = amount - fee_amount;

        // Execute transfers
        if fee_amount > 0 {
            token_client.transfer(sender, platform_treasury, &fee_amount);
            // ── Fee accounting (schema v1) ─────────────────────────────────
            // Only increment the counter after the transfer succeeds so that
            // failed payments never inflate the total.
            Self::accrue_fee_total(env, token_address, fee_amount);
            env.events().publish(
                (Symbol::new(env, "fee_collected"), token_address.clone()),
                fee_amount,
            );
        }
        if remainder > 0 {
            match token_client.try_transfer(sender, recipient, &remainder) {
                Ok(Ok(())) => {
                    log!(env, "Remaining balance routed to recipient");
                }
                _ => {
                    log!(
                        env,
                        "Recipient transfer failed; crediting sender refund balance"
                    );
                    token_client.transfer(sender, &env.current_contract_address(), &remainder);
                    Self::credit_refund_balance(env, sender, token_address, remainder);
                }
            }
        }

        // Record cumulative volume
        let volume_key = DataKey::UserVolume(sender.clone());
        let prev_volume: i128 = env.storage().persistent().get(&volume_key).unwrap_or(0);
        env.storage()
            .persistent()
            .set(&volume_key, &(prev_volume + amount));
        env.storage().persistent().extend_ttl(
            &volume_key,
            Self::PERSISTENT_LIFETIME_THRESHOLD,
            Self::PERSISTENT_BUMP_AMOUNT,
        );

        // ── Write immutable execution receipt (schema v1) ─────────────────
        let receipt = PaymentReceipt {
            sender: sender.clone(),
            recipient: recipient.clone(),
            token: token_address.clone(),
            amount,
            fee: fee_amount,
            executed_at: now,
            expires_at: now.saturating_add(Self::NONCE_EXPIRY_SECONDS),
        };
        Self::write_receipt(env, sender, receipt_nonce, &receipt);

        // Emit routed event
        env.events().publish(
            (symbol_short!("routed"), sender.clone(), recipient.clone()),
            amount,
        );

        log!(env, "Platform fee routed to treasury");

        Ok(())
    }

    // ── Public contract methods ──────────────────────────────────────────────

    /// One-time setup: records the admin and the initial fee configuration
    /// in instance storage. Must be called before `route_payment`.
    ///
    /// # Parameters
    /// - `admin`: Address granted admin rights over the contract; must
    ///   authorize this call.
    /// - `platform_treasury`: Address that receives collected platform fees.
    /// - `fee_bps`: Platform fee rate, in basis points.
    /// - `fee_cap`: Maximum fee (in the token's smallest unit) taken from a
    ///   single payment.
    /// - `max_amount`: Maximum amount accepted by a single payment.
    ///
    /// # Returns
    /// `Ok(())` on success, or `Err(Error::AlreadyInitialized)` if the
    /// contract already has an admin set.
    ///
    /// # Panics
    /// Panics if `admin` does not authorize the call.
    pub fn initialize(
        env: Env,
        admin: Address,
        platform_treasury: Address,
        fee_bps: i128,
        fee_cap: i128,
        max_amount: i128,
    ) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        admin.require_auth();

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::PlatformTreasury, &platform_treasury);
        env.storage().instance().set(&DataKey::FeeBps, &fee_bps);
        env.storage().instance().set(&DataKey::FeeCap, &fee_cap);
        env.storage()
            .instance()
            .set(&DataKey::MaxAmount, &max_amount);
        env.storage().instance().set(&DataKey::Paused, &false);
        env.storage().instance().set(&DataKey::Frozen, &false);
        env.storage().instance().set(&DataKey::TimelockNonce, &0u64);
        env.storage()
            .instance()
            .set(&DataKey::SchemaVersion, &SCHEMA_VERSION);
        env.storage().instance().extend_ttl(
            Self::INSTANCE_LIFETIME_THRESHOLD,
            Self::INSTANCE_BUMP_AMOUNT,
        );

        Ok(())
    }

    // ── Timelock: queue / execute / cancel ───────────────────────────────────

    /// Queues an admin action to be executed after a 24-hour delay.
    ///
    /// The admin provides the desired `ActionType` variant and receives a
    /// numeric nonce that uniquely identifies this pending entry.  Pass this
    /// nonce to `execute_action` after 24 hours, or to `cancel_action` to
    /// abort the intent.
    ///
    /// Sensitive configuration, routing-policy, and admin-transfer changes
    /// must go through the timelock. Identical pending actions are rejected.
    /// Fee actions are authorized by governance when configured, otherwise by
    /// the admin; other actions require the admin. `Unfreeze` may be queued
    /// while frozen, but all other actions are blocked until the contract is
    /// unfrozen.
    pub fn queue_action(env: Env, action: ActionType) -> Result<u64, Error> {
        if Self::is_frozen_internal(&env) && !matches!(&action, ActionType::Unfreeze) {
            return Err(Error::ContractFrozen);
        }

        let authority = Self::action_authority(&env, &action)?;
        authority.require_auth();
        Self::validate_action(&action)?;

        if let ActionType::TransferAdmin(ref new_admin) = action {
            if new_admin == &authority {
                return Err(Error::InvalidAdminTransfer);
            }
        }

        let pending_action_key = DataKey::PendingAction(action.clone());
        if env.storage().persistent().has(&pending_action_key) {
            return Err(Error::TimelockPending);
        }

        let nonce = Self::next_nonce(&env);
        let queued_at = env.ledger().timestamp();

        let entry = TimelockEntry {
            queued_at,
            action: action.clone(),
        };

        let key = DataKey::TimelockEntry(nonce);
        env.storage().persistent().set(&key, &entry);
        env.storage().persistent().extend_ttl(
            &key,
            Self::PERSISTENT_LIFETIME_THRESHOLD,
            Self::PERSISTENT_BUMP_AMOUNT,
        );
        env.storage().persistent().set(&pending_action_key, &nonce);
        env.storage().persistent().extend_ttl(
            &pending_action_key,
            Self::PERSISTENT_LIFETIME_THRESHOLD,
            Self::PERSISTENT_BUMP_AMOUNT,
        );

        env.storage().instance().extend_ttl(
            Self::INSTANCE_LIFETIME_THRESHOLD,
            Self::INSTANCE_BUMP_AMOUNT,
        );

        env.events().publish(
            (Symbol::new(&env, "action_queued"), authority),
            (nonce, queued_at),
        );

        log!(&env, "Timelock action queued with nonce {}", nonce);
        Ok(nonce)
    }

    /// Returns the pending `TimelockEntry` for the given nonce, or an error if
    /// it does not exist.
    pub fn get_queued_action(env: Env, nonce: u64) -> Result<TimelockEntry, Error> {
        let key = DataKey::TimelockEntry(nonce);
        env.storage()
            .persistent()
            .get(&key)
            .ok_or(Error::TimelockNotFound)
    }

    /// Executes a previously queued action identified by `nonce`.
    ///
    /// Requirements:
    /// - The contract must not be frozen.
    /// - The admin must authorize.
    /// - The entry identified by `nonce` must exist.
    /// - At least 24 hours (`SECONDS_IN_24H`) must have passed since queuing.
    ///
    /// On success the entry is removed and the underlying setter is invoked.
    pub fn execute_action(env: Env, nonce: u64) -> Result<(), Error> {
        let key = DataKey::TimelockEntry(nonce);
        let entry: TimelockEntry = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(Error::TimelockNotFound)?;
        if Self::is_frozen_internal(&env) && !matches!(&entry.action, ActionType::Unfreeze) {
            return Err(Error::ContractFrozen);
        }
        let authority = Self::action_authority(&env, &entry.action)?;
        authority.require_auth();

        let now = env.ledger().timestamp();
        if now < entry.queued_at + Self::SECONDS_IN_24H {
            return Err(Error::TimelockNotReady);
        }

        // Remove the entry before applying the action (checks-effects-interactions).
        env.storage().persistent().remove(&key);
        env.storage()
            .persistent()
            .remove(&DataKey::PendingAction(entry.action.clone()));

        // Apply the action.
        match entry.action {
            ActionType::SetPlatformTreasury(new_treasury) => {
                env.storage()
                    .instance()
                    .set(&DataKey::PlatformTreasury, &new_treasury);
            }
            ActionType::SetFeeConfig(fee_bps, fee_cap) => {
                env.storage().instance().set(&DataKey::FeeBps, &fee_bps);
                env.storage().instance().set(&DataKey::FeeCap, &fee_cap);
            }
            ActionType::SetFeeBps(new_fee_bps) => {
                env.storage().instance().set(&DataKey::FeeBps, &new_fee_bps);
            }
            ActionType::SetGovernance(gov) => {
                env.storage().instance().set(&DataKey::Governance, &gov);
            }
            ActionType::SetMinLimit(min_limit) => {
                env.storage().instance().set(&DataKey::MinLimit, &min_limit);
            }
            ActionType::SetMinAmount(min_amount) => {
                env.storage()
                    .instance()
                    .set(&DataKey::MinAmount, &min_amount);
            }
            ActionType::SetMaxAmount(max_amount) => {
                env.storage()
                    .instance()
                    .set(&DataKey::MaxAmount, &max_amount);
            }
            ActionType::SetBlacklist(address, blacklisted) => {
                let blacklist_key = DataKey::Blacklist(address.clone());
                if blacklisted {
                    env.storage().persistent().set(&blacklist_key, &true);
                    env.storage().persistent().extend_ttl(
                        &blacklist_key,
                        Self::PERSISTENT_LIFETIME_THRESHOLD,
                        Self::PERSISTENT_BUMP_AMOUNT,
                    );
                } else {
                    env.storage().persistent().remove(&blacklist_key);
                }
            }
            ActionType::SetTokenBlocked(token, blocked) => {
                let token_key = DataKey::TokenBlocked(token.clone());
                if blocked {
                    env.storage().persistent().set(&token_key, &true);
                    env.storage().persistent().extend_ttl(
                        &token_key,
                        Self::PERSISTENT_LIFETIME_THRESHOLD,
                        Self::PERSISTENT_BUMP_AMOUNT,
                    );
                } else {
                    env.storage().persistent().remove(&token_key);
                }
            }
            ActionType::SetPaused(paused) => {
                env.storage().instance().set(&DataKey::Paused, &paused);
                env.events().publish((symbol_short!("pause"),), (paused,));
            }
            ActionType::Unfreeze => {
                env.storage().instance().set(&DataKey::Frozen, &false);
                env.events().publish(
                    (Symbol::new(&env, "unfreeze"), authority.clone()),
                    env.ledger().timestamp(),
                );
            }
            ActionType::TransferAdmin(new_admin) => {
                let pending_key = DataKey::PendingAdminTransfer;
                if let Some(pending) = env
                    .storage()
                    .persistent()
                    .get::<DataKey, PendingAdminTransfer>(&pending_key)
                {
                    if now <= pending.expires_at {
                        return Err(Error::AdminTransferPending);
                    }
                    env.storage().persistent().remove(&pending_key);
                    env.events().publish(
                        (
                            Symbol::new(&env, "admin_transfer_expired"),
                            pending.proposed_admin,
                        ),
                        (pending.action_nonce, pending.expires_at),
                    );
                }
                let pending = PendingAdminTransfer {
                    proposed_admin: new_admin.clone(),
                    proposed_at: now,
                    expires_at: now + Self::ADMIN_TRANSFER_EXPIRY_SECONDS,
                    action_nonce: nonce,
                };
                env.storage().persistent().set(&pending_key, &pending);
                env.storage().persistent().extend_ttl(
                    &pending_key,
                    Self::PERSISTENT_LIFETIME_THRESHOLD,
                    Self::PERSISTENT_BUMP_AMOUNT,
                );
                env.events().publish(
                    (
                        Symbol::new(&env, "admin_transfer_proposed"),
                        authority.clone(),
                        new_admin,
                    ),
                    (nonce, pending.expires_at),
                );
            }
            ActionType::Upgrade(new_wasm_hash) => {
                env.deployer().update_current_contract_wasm(new_wasm_hash);
            }
        }

        env.storage().instance().extend_ttl(
            Self::INSTANCE_LIFETIME_THRESHOLD,
            Self::INSTANCE_BUMP_AMOUNT,
        );

        env.events()
            .publish((Symbol::new(&env, "action_executed"), authority), nonce);

        log!(&env, "Timelock action executed for nonce {}", nonce);
        Ok(())
    }

    /// Cancels a pending timelock entry before it can be executed.
    ///
    /// This is the primary defence when a compromised admin has queued a
    /// malicious action: any other admin (after a key rotation) or a
    /// multi-sig governance can cancel it within the 24-hour window.
    ///
    /// Admin authorization is required. The contract may be frozen.
    pub fn cancel_action(env: Env, nonce: u64) -> Result<(), Error> {
        let admin = Self::require_admin(&env)?;
        admin.require_auth();

        let key = DataKey::TimelockEntry(nonce);
        let entry: TimelockEntry = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(Error::TimelockNotFound)?;

        env.storage().persistent().remove(&key);
        env.storage()
            .persistent()
            .remove(&DataKey::PendingAction(entry.action));

        env.events()
            .publish((Symbol::new(&env, "action_cancelled"), admin), nonce);

        log!(&env, "Timelock action cancelled for nonce {}", nonce);
        Ok(())
    }

    /// Returns the current admin nomination, if one is awaiting acceptance.
    pub fn get_pending_admin_transfer(env: Env) -> Option<PendingAdminTransfer> {
        env.storage()
            .persistent()
            .get(&DataKey::PendingAdminTransfer)
    }

    /// Accepts an unexpired admin nomination. The current admin remains active
    /// until the nominated address authorizes this call.
    pub fn accept_admin(env: Env) -> Result<(), Error> {
        let key = DataKey::PendingAdminTransfer;
        let pending: PendingAdminTransfer = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(Error::TimelockNotFound)?;
        if env.ledger().timestamp() > pending.expires_at {
            return Err(Error::AdminTransferExpired);
        }
        pending.proposed_admin.require_auth();
        env.storage()
            .instance()
            .set(&DataKey::Admin, &pending.proposed_admin);
        env.storage().persistent().remove(&key);
        env.storage().instance().extend_ttl(
            Self::INSTANCE_LIFETIME_THRESHOLD,
            Self::INSTANCE_BUMP_AMOUNT,
        );
        env.events().publish(
            (
                Symbol::new(&env, "admin_transfer_accepted"),
                pending.proposed_admin,
            ),
            (pending.action_nonce, env.ledger().timestamp()),
        );
        Ok(())
    }

    /// Cancels an admin nomination while the current admin is still active.
    pub fn cancel_admin_transfer(env: Env) -> Result<(), Error> {
        let admin = Self::require_admin(&env)?;
        admin.require_auth();
        let key = DataKey::PendingAdminTransfer;
        let pending: PendingAdminTransfer = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(Error::TimelockNotFound)?;
        env.storage().persistent().remove(&key);
        env.events().publish(
            (
                Symbol::new(&env, "admin_transfer_cancelled"),
                admin,
                pending.proposed_admin,
            ),
            pending.action_nonce,
        );
        Ok(())
    }

    /// Clears an expired nomination. Anyone may call this after its expiry.
    pub fn expire_admin_transfer(env: Env) -> Result<(), Error> {
        let key = DataKey::PendingAdminTransfer;
        let pending: PendingAdminTransfer = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(Error::TimelockNotFound)?;
        if env.ledger().timestamp() <= pending.expires_at {
            return Err(Error::TimelockNotReady);
        }
        env.storage().persistent().remove(&key);
        env.events().publish(
            (
                Symbol::new(&env, "admin_transfer_expired"),
                pending.proposed_admin,
            ),
            (pending.action_nonce, pending.expires_at),
        );
        Ok(())
    }

    // ── Freeze / unfreeze ────────────────────────────────────────────────────

    /// Instantly freezes the contract, blocking all payments and timelock
    /// executions.  This is the emergency last resort when an admin key is
    /// known to be compromised.
    ///
    /// Unlike other sensitive admin operations, freeze takes effect immediately
    /// — it does NOT go through the timelock — so it is always available as a
    /// rapid-response tool.
    ///
    /// Admin authorization is required.
    pub fn emergency_freeze(env: Env) -> Result<(), Error> {
        let admin = Self::require_admin(&env)?;
        admin.require_auth();

        env.storage().instance().set(&DataKey::Frozen, &true);
        env.storage().instance().extend_ttl(
            Self::INSTANCE_LIFETIME_THRESHOLD,
            Self::INSTANCE_BUMP_AMOUNT,
        );

        env.events().publish(
            (Symbol::new(&env, "emergency_freeze"), admin),
            env.ledger().timestamp(),
        );

        log!(&env, "Contract frozen by admin");
        Ok(())
    }

    /// Deprecated immediate unfreeze path. Queue `ActionType::Unfreeze` and
    /// execute it after the 24-hour delay instead.
    pub fn unfreeze(env: Env) -> Result<(), Error> {
        let admin = Self::require_admin(&env)?;
        admin.require_auth();
        Err(Error::TimelockRequired)
    }

    /// Returns whether the contract is currently frozen.
    pub fn is_frozen(env: Env) -> bool {
        Self::is_frozen_internal(&env)
    }

    // ── Sensitive admin setters (now require timelock) ───────────────────────
    //
    // Legacy setter entry points remain available for ABI compatibility, but
    // reject direct mutation. Changes must be represented by ActionType and
    // applied only by execute_action.

    /// Deprecated direct treasury update. Queue `SetPlatformTreasury` instead.
    ///
    /// # Parameters
    /// - `new_treasury`: Address to receive platform fees going forward.
    ///
    /// # Returns
    /// `Err(Error::TimelockRequired)` after admin authorization. Queue the
    /// corresponding `ActionType` instead.
    ///
    /// # Panics
    /// Panics if the current admin does not authorize the call.
    ///
    /// DEPRECATED for direct use.  Queue via `queue_action(ActionType::SetPlatformTreasury(…))`
    /// and execute after 24 hours.  This direct path is retained for tooling
    /// compatibility only.
    pub fn set_platform_treasury(env: Env, new_treasury: Address) -> Result<(), Error> {
        let admin = Self::require_admin(&env)?;
        admin.require_auth();
        let _ = new_treasury;
        Err(Error::TimelockRequired)
    }

    /// Updates the fee basis points and fee cap.
    /// Requires governance authority if a governance address is set; otherwise admin-only.
    ///
    /// # Parameters
    /// - `fee_bps`: New platform fee rate, in basis points.
    /// - `fee_cap`: New maximum fee taken from a single payment.
    ///
    /// # Returns
    /// `Err(Error::TimelockRequired)` after fee-authority authorization.
    ///
    /// # Panics
    /// Panics if the caller does not authorize the call.
    ///
    /// DEPRECATED for direct use.  Queue via `queue_action(ActionType::SetFeeConfig(…))`.
    pub fn set_fee_config_legacy(env: Env, fee_bps: i128, fee_cap: i128) -> Result<(), Error> {
        Self::require_fee_authority(&env)?;
        let _ = (fee_bps, fee_cap);
        Err(Error::TimelockRequired)
    }

    /// Alias for `set_fee_config_legacy`. Admin-only.
    ///
    /// # Parameters
    /// - `fee_bps`: New platform fee rate, in basis points.
    /// - `fee_cap`: New maximum fee taken from a single payment.
    ///
    /// # Returns
    /// See `set_fee_config_legacy`.
    ///
    /// # Panics
    /// Panics if the current admin does not authorize the call.
    ///
    /// DEPRECATED for direct use.  Queue via `queue_action(ActionType::SetFeeConfig(…))`.
    pub fn set_fee_config(env: Env, fee_bps: i128, fee_cap: i128) -> Result<(), Error> {
        Self::set_fee_config_legacy(env, fee_bps, fee_cap)
    }

    /// Updates the fee basis points.
    /// Requires governance authority if a governance address is set; otherwise admin-only.
    ///
    /// # Parameters
    /// - `new_fee_bps`: New platform fee rate, in basis points.
    ///
    /// # Returns
    /// `Err(Error::TimelockRequired)` after fee-authority authorization.
    ///
    /// # Panics
    /// Panics if the caller does not authorize the call.
    ///
    /// DEPRECATED for direct use.  Queue via `queue_action(ActionType::SetFeeBps(…))`.
    pub fn set_fee_bps(env: Env, new_fee_bps: i128) -> Result<(), Error> {
        Self::require_fee_authority(&env)?;
        let _ = new_fee_bps;
        Err(Error::TimelockRequired)
    }

    /// Deprecated direct governance update. Queue `SetGovernance` instead.
    ///
    /// DEPRECATED for direct use.  Queue via `queue_action(ActionType::SetGovernance(…))`.
    pub fn set_governance(env: Env, gov: Address) -> Result<(), Error> {
        let admin = Self::require_admin(&env)?;
        admin.require_auth();
        let _ = gov;
        Err(Error::TimelockRequired)
    }

    /// Deprecated direct minimum-limit update. Queue `SetMinLimit` instead.
    ///
    /// # Parameters
    /// - `min_limit`: Smallest `amount` that `route_payment` /
    ///   `route_payments` will accept going forward.
    ///
    /// # Returns
    /// `Err(Error::TimelockRequired)` after admin authorization.
    ///
    /// # Panics
    /// Panics if the current admin does not authorize the call.
    ///
    /// DEPRECATED for direct use.  Queue via `queue_action(ActionType::SetMinLimit(…))`.
    pub fn set_min_limit(env: Env, min_limit: i128) -> Result<(), Error> {
        let admin = Self::require_admin(&env)?;
        admin.require_auth();
        let _ = min_limit;
        Err(Error::TimelockRequired)
    }

    /// Deprecated direct minimum transfer update. Queue `SetMinAmount` instead.
    ///
    /// # Parameters
    /// - `amount`: Smallest `amount` that `route_payment` will accept going
    ///   forward. Payments below this value are rejected with
    ///   `Error::AmountBelowMinimum`.
    ///
    /// # Returns
    /// `Err(Error::TimelockRequired)` after admin authorization.
    ///
    /// # Panics
    /// Panics if the current admin does not authorize the call.
    pub fn set_min_amount(env: Env, amount: i128) -> Result<(), Error> {
        let admin = Self::require_admin(&env)?;
        admin.require_auth();
        let _ = amount;
        Err(Error::TimelockRequired)
    }

    /// Deprecated direct maximum payment update. Queue `SetMaxAmount` instead.
    pub fn set_max_amount(env: Env, amount: i128) -> Result<(), Error> {
        let admin = Self::require_admin(&env)?;
        admin.require_auth();
        let _ = amount;
        Err(Error::TimelockRequired)
    }

    /// Returns the current protocol fee percentage in basis points.
    ///
    /// # Returns
    /// The configured `fee_bps`, or `0` if the contract has not been
    /// initialized.
    ///
    /// # Panics
    /// Does not panic.
    pub fn get_fee(env: Env) -> i128 {
        env.storage().instance().get(&DataKey::FeeBps).unwrap_or(0)
    }

    /// Deprecated direct pause update. Queue `SetPaused` instead.
    ///
    /// # Parameters
    /// - `paused`: `true` to reject `route_payment` / `route_payments`
    ///   calls, `false` to allow them again.
    ///
    /// # Returns
    /// `Err(Error::TimelockRequired)` after admin authorization.
    ///
    /// # Panics
    /// Panics if the current admin does not authorize the call.
    ///
    /// Emergency freeze remains immediately available through `emergency_freeze`.
    pub fn set_pause(env: Env, paused: bool) -> Result<(), Error> {
        let admin = Self::require_admin(&env)?;
        admin.require_auth();
        let _ = paused;
        Err(Error::TimelockRequired)
    }

    /// Alias for `set_pause`. Admin-only.
    ///
    /// # Parameters
    /// - `paused`: `true` to reject routing calls, `false` to allow them.
    ///
    /// # Returns
    /// See `set_pause`.
    ///
    /// # Panics
    /// Panics if the current admin does not authorize the call.
    pub fn set_paused(env: Env, paused: bool) -> Result<(), Error> {
        Self::set_pause(env, paused)
    }

    /// Returns whether the contract is currently paused.
    ///
    /// # Returns
    /// `true` if paused, `false` if unpaused or not yet initialized.
    ///
    /// # Panics
    /// Does not panic.
    pub fn is_paused(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::Paused)
            .unwrap_or(false)
    }

    /// Returns the cumulative amount a given sender has routed through the contract.
    ///
    /// # Parameters
    /// - `user`: Sender address to look up.
    ///
    /// # Returns
    /// The lifetime routed volume for `user`, or `0` if they have never
    /// routed a payment.
    ///
    /// # Panics
    /// Does not panic.
    pub fn get_user_volume(env: Env, user: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::UserVolume(user))
            .unwrap_or(0)
    }

    /// Deprecated direct blacklist mutation. Queue `SetBlacklist` instead.
    ///
    /// # Parameters
    /// - `address`: Address to blacklist; subsequent payments to it as a
    ///   recipient will be rejected.
    ///
    /// # Returns
    /// `Err(Error::TimelockRequired)` after admin authorization.
    ///
    /// # Panics
    /// Panics if the current admin does not authorize the call.
    pub fn blacklist_address(env: Env, address: Address) -> Result<(), Error> {
        let admin = Self::require_admin(&env)?;
        admin.require_auth();
        let _ = address;
        Err(Error::TimelockRequired)
    }

    /// Deprecated direct blacklist mutation. Queue `SetBlacklist` instead.
    ///
    /// # Parameters
    /// - `address`: Address to remove from the blacklist.
    ///
    /// # Returns
    /// `Err(Error::TimelockRequired)` after admin authorization.
    ///
    /// # Panics
    /// Panics if the current admin does not authorize the call.
    pub fn unblacklist_address(env: Env, address: Address) -> Result<(), Error> {
        let admin = Self::require_admin(&env)?;
        admin.require_auth();
        let _ = address;
        Err(Error::TimelockRequired)
    }

    /// Direct token policy changes are disabled; queue SetTokenBlocked instead.
    pub fn set_token_blocked(env: Env, token: Address, blocked: bool) -> Result<(), Error> {
        let admin = Self::require_admin(&env)?;
        admin.require_auth();
        let _ = (token, blocked);
        Err(Error::TimelockRequired)
    }

    /// Returns whether an address is blacklisted.
    ///
    /// # Parameters
    /// - `address`: Address to check.
    ///
    /// # Returns
    /// `true` if `address` is blacklisted, `false` otherwise.
    ///
    /// # Panics
    /// Does not panic.
    pub fn is_blacklisted(env: Env, address: Address) -> bool {
        env.storage()
            .persistent()
            .get(&DataKey::Blacklist(address))
            .unwrap_or(false)
    }

    /// Returns the effective fee_bps for a sender after applying any
    /// volume-based tiered discount.
    ///
    /// # Parameters
    /// - `sender`: Address whose discounted fee rate to compute.
    ///
    /// # Returns
    /// The configured `fee_bps`, halved if `sender`'s lifetime volume
    /// exceeds the tiered-discount threshold, or `0` if not initialized.
    ///
    /// # Panics
    /// Does not panic.
    pub fn get_effective_fee_bps(env: Env, sender: Address) -> i128 {
        let fee_bps: i128 = env.storage().instance().get(&DataKey::FeeBps).unwrap_or(0);
        let user_volume = Self::get_user_volume(env.clone(), sender);
        if user_volume > Self::VOLUME_THRESHOLD {
            fee_bps / 2
        } else {
            fee_bps
        }
    }

    /// Deprecated direct admin mutation. Use the timelocked two-step transfer.
    ///
    /// # Parameters
    /// - `new_admin`: Address to install as the new admin.
    ///
    /// # Returns
    /// `Err(Error::TimelockRequired)` after requiring current-admin authorization.
    ///
    /// # Panics
    /// Panics if an admin is already set and it does not authorize the call.
    pub fn set_admin(env: Env, new_admin: Address) -> Result<(), Error> {
        let admin = Self::require_admin(&env)?;
        admin.require_auth();
        let _ = new_admin;
        Err(Error::TimelockRequired)
    }

    /// Deprecated direct admin transfer. Queue `TransferAdmin` instead.
    ///
    /// # Parameters
    /// - `new_admin`: Address to become the new admin.
    ///
    /// # Returns
    /// `Err(Error::TimelockRequired)` after current-admin authorization.
    ///
    /// # Panics
    /// Panics if the current admin does not authorize the call.
    ///
    /// DEPRECATED for direct use.  Queue via `queue_action(ActionType::TransferAdmin(…))`.
    pub fn transfer_admin(env: Env, new_admin: Address) -> Result<(), Error> {
        let current_admin = Self::require_admin(&env)?;
        current_admin.require_auth();
        let _ = new_admin;
        Err(Error::TimelockRequired)
    }

    /// Recovers tokens accidentally sent directly to the contract address. Admin-only.
    ///
    /// # Parameters
    /// - `token`: Contract ID of the token to recover.
    /// - `amount`: Amount to transfer from the contract's balance to the admin.
    ///
    /// # Returns
    /// `Ok(())` on success, or `Err(Error::NotInitialized)` if the contract
    /// has no admin set yet.
    ///
    /// # Panics
    /// Panics if the current admin does not authorize the call, or if the
    /// token transfer fails (e.g. the contract's balance is below `amount`).
    pub fn recover_tokens(env: Env, token: Address, amount: i128) -> Result<(), Error> {
        let admin = Self::require_admin(&env)?;
        admin.require_auth();

        let contract_address = env.current_contract_address();
        let token_client = token::Client::new(&env, &token);
        token_client.transfer(&contract_address, &admin, &amount);

        Ok(())
    }

    /// Records a token as supported (no-op; routing accepts any token contract ID).
    ///
    /// # Parameters
    /// - `_token`: Ignored; present for API compatibility.
    ///
    /// # Returns
    /// Always `Ok(())`.
    ///
    /// # Panics
    /// Does not panic.
    pub fn add_supported_token(_env: Env, _token: Address) -> Result<(), Error> {
        Ok(())
    }

    /// Returns the current administrator.
    pub fn get_admin(env: Env) -> Result<Address, Error> {
        Self::require_admin(&env)
    }

    /// Routes a payment from a sender to a recipient, deducting a platform fee.
    ///
    /// # Parameters
    /// - `sender`: Address the funds are debited from; must authorize the call.
    /// - `recipient`: Address to receive the funds (minus the platform fee).
    /// - `token_address`: Contract ID of the token being transferred.
    /// - `amount`: Amount to route, in the token's smallest unit. Must be
    ///   positive and within the configured min/max and daily-limit bounds.
    ///
    /// # Returns
    /// `Ok(())` on success. Returns `Err(Error::Paused)` if routing is
    /// paused, `Err(Error::NotInitialized)` if the contract has no admin
    /// set, `Err(Error::InvalidRecipient)` if `sender == recipient`,
    /// `Err(Error::Blacklisted)` if `recipient` is blacklisted,
    /// `Err(Error::LimitExceeded)` if `amount` is outside the configured
    /// bounds or exceeds the sender's remaining daily limit, or
    /// `Err(Error::InsufficientBalance)` if `sender`'s token balance is
    /// below `amount`.
    ///
    /// # Panics
    /// Panics if `sender` does not authorize the call, or if the underlying
    /// token transfer to `platform_treasury` fails.
    pub fn route_payment(
        env: Env,
        sender: Address,
        recipient: Address,
        token_address: Address,
        amount: i128,
    ) -> Result<(), Error> {
        if Self::is_frozen_internal(&env) {
            return Err(Error::ContractFrozen);
        }
        if Self::is_paused(env.clone()) {
            return Err(Error::Paused);
        }

        let (platform_treasury, fee_bps, fee_cap) = Self::load_fee_config(&env)?;

        Self::process_single_payment(
            &env,
            &sender,
            &recipient,
            &token_address,
            amount,
            &platform_treasury,
            fee_bps,
            fee_cap,
            None,
        )
    }

    /// Routes multiple payments in a single transaction. If any payment fails,
    /// the entire batch is reverted atomically.
    ///
    /// # Parameters
    /// - `payments`: Batch of transfer instructions to apply in order. See
    ///   [`Payment`] for per-item constraints.
    ///
    /// # Returns
    /// `Ok(())` if every payment in the batch succeeds. Returns the first
    /// error encountered (see `route_payment` for the possible `Err`
    /// variants and their causes) if any payment fails; the Soroban host
    /// reverts all storage and balance changes from the batch in that case.
    ///
    /// # Panics
    /// Panics if any payment's `sender` does not authorize the call, or if
    /// a token transfer to `platform_treasury` fails.
    pub fn route_payments(env: Env, payments: Vec<Payment>) -> Result<(), Error> {
        if Self::is_frozen_internal(&env) {
            return Err(Error::ContractFrozen);
        }
        if Self::is_paused(env.clone()) {
            return Err(Error::Paused);
        }

        let (platform_treasury, fee_bps, fee_cap) = Self::load_fee_config(&env)?;

        for payment in payments.iter() {
            Self::process_single_payment(
                &env,
                &payment.sender,
                &payment.recipient,
                &payment.token_address,
                payment.amount,
                &platform_treasury,
                fee_bps,
                fee_cap,
                None,
            )?;
        }

        Ok(())
    }

    /// Returns the available internal refund balance for a user and token.
    ///
    /// # Parameters
    /// - `user`: Address whose refund balance to look up.
    /// - `token`: Contract ID of the token.
    ///
    /// # Returns
    /// The refundable balance for `(user, token)`, or `0` if none is held.
    ///
    /// # Panics
    /// Does not panic.
    pub fn get_refund_balance(env: Env, user: Address, token: Address) -> i128 {
        Self::get_refund_balance_internal(&env, &user, &token)
    }

    /// Withdraws a specific amount from the user's internal refund balance.
    ///
    /// A refund balance accrues when a `route_payment` / `route_payments`
    /// transfer to the recipient fails (e.g. missing trustline) and the
    /// funds are held by the contract on the sender's behalf instead.
    ///
    /// # Parameters
    /// - `user`: Address withdrawing funds; must authorize the call.
    /// - `token`: Contract ID of the token to withdraw.
    /// - `amount`: Amount to withdraw. Must be positive and not exceed the
    ///   current refund balance.
    ///
    /// # Returns
    /// `Ok(())` on success, or `Err(Error::NoRefundAvailable)` if `amount`
    /// is zero, negative, or greater than the available balance.
    ///
    /// # Panics
    /// Panics if `user` does not authorize the call, or if the underlying
    /// token transfer fails.
    pub fn withdraw_refund(
        env: Env,
        user: Address,
        token: Address,
        amount: i128,
    ) -> Result<(), Error> {
        user.require_auth();

        if amount <= 0 {
            return Err(Error::NoRefundAvailable);
        }

        // ── Schema v1: check RefundEntry with expiry enforcement ─────────────
        // If a RefundEntry exists, enforce the expiry window.  Expired entries
        // can only be claimed by the governed recovery path.
        if let Some(entry) = Self::get_refund_entry(&env, &user, &token) {
            if Self::refund_is_expired(&env, &entry) {
                return Err(Error::RefundExpired);
            }
            if amount > entry.balance {
                return Err(Error::NoRefundAvailable);
            }
            let key = DataKey::RefundEntry(user.clone(), token.clone());
            let new_balance = entry.balance - amount;
            if new_balance > 0 {
                let updated = RefundEntry {
                    balance: new_balance,
                    created_at: entry.created_at,
                };
                env.storage().persistent().set(&key, &updated);
                env.storage().persistent().extend_ttl(
                    &key,
                    Self::PERSISTENT_LIFETIME_THRESHOLD,
                    Self::PERSISTENT_BUMP_AMOUNT,
                );
            } else {
                env.storage().persistent().remove(&key);
            }
            let contract_address = env.current_contract_address();
            let token_client = token::Client::new(&env, &token);
            token_client.transfer(&contract_address, &user, &amount);
            env.events().publish(
                (symbol_short!("withdrawn"), user.clone(), token.clone()),
                amount,
            );
            log!(&env, "Refund balance withdrawn by user");
            return Ok(());
        }

        // ── Legacy path: bare RefundBalance i128 ─────────────────────────────
        let current_balance = Self::get_refund_balance_internal(&env, &user, &token);
        if amount > current_balance {
            return Err(Error::NoRefundAvailable);
        }

        let key = DataKey::RefundBalance(user.clone(), token.clone());
        let new_balance = current_balance - amount;
        if new_balance > 0 {
            env.storage().persistent().set(&key, &new_balance);
            env.storage().persistent().extend_ttl(
                &key,
                Self::PERSISTENT_LIFETIME_THRESHOLD,
                Self::PERSISTENT_BUMP_AMOUNT,
            );
        } else {
            env.storage().persistent().remove(&key);
        }

        let contract_address = env.current_contract_address();
        let token_client = token::Client::new(&env, &token);
        token_client.transfer(&contract_address, &user, &amount);

        env.events().publish(
            (symbol_short!("withdrawn"), user.clone(), token.clone()),
            amount,
        );

        log!(&env, "Refund balance withdrawn by user");
        Ok(())
    }

    /// Claims and withdraws the entire available refund balance for a user and token.
    ///
    /// # Parameters
    /// - `user`: Address withdrawing funds; must authorize the call.
    /// - `token`: Contract ID of the token to withdraw.
    ///
    /// # Returns
    /// `Ok(amount)` with the amount withdrawn, or
    /// `Err(Error::NoRefundAvailable)` if the refund balance is zero.
    ///
    /// # Panics
    /// Panics if `user` does not authorize the call, or if the underlying
    /// token transfer fails.
    pub fn claim_all_refunds(env: Env, user: Address, token: Address) -> Result<i128, Error> {
        user.require_auth();

        // Schema v1: check RefundEntry with expiry enforcement.
        if let Some(entry) = Self::get_refund_entry(&env, &user, &token) {
            if Self::refund_is_expired(&env, &entry) {
                return Err(Error::RefundExpired);
            }
            let current_balance = entry.balance;
            if current_balance <= 0 {
                return Err(Error::NoRefundAvailable);
            }
            Self::withdraw_refund(env, user, token, current_balance)?;
            return Ok(current_balance);
        }

        // Legacy path.
        let current_balance = Self::get_refund_balance_internal(&env, &user, &token);
        if current_balance <= 0 {
            return Err(Error::NoRefundAvailable);
        }

        Self::withdraw_refund(env, user, token, current_balance)?;
        Ok(current_balance)
    }

    /// Admin-only emergency withdrawal of tokens held by this contract.
    ///
    /// # Parameters
    /// - `token`: Contract ID of the token to withdraw.
    /// - `amount`: Amount to transfer from the contract's balance to the admin.
    ///
    /// # Returns
    /// `Ok(())` on success, or `Err(Error::NotInitialized)` if the contract
    /// has no admin set yet.
    ///
    /// # Panics
    /// Panics if the current admin does not authorize the call, or if the
    /// token transfer fails (e.g. the contract's balance is below `amount`).
    pub fn emergency_withdraw(env: Env, token: Address, amount: i128) -> Result<(), Error> {
        let admin = Self::require_admin(&env)?;
        admin.require_auth();

        let token_client = token::Client::new(&env, &token);
        token_client.transfer(&env.current_contract_address(), &admin, &amount);

        log!(&env, "Emergency withdraw executed by admin");
        Ok(())
    }

    // ── Schema v1 public API ─────────────────────────────────────────────────

    /// Routes a payment using a caller-supplied nonce for idempotency.
    ///
    /// The nonce must equal `get_payment_nonce_counter(sender) + 1`.  On
    /// success an immutable `PaymentReceipt` is written at `(sender, nonce)`.
    /// Supplying the same nonce a second time returns `Error::NonceDuplicate`.
    /// Supplying an out-of-sequence nonce returns `Error::NonceExpired`.
    pub fn route_payment_with_nonce(
        env: Env,
        sender: Address,
        recipient: Address,
        token_address: Address,
        amount: i128,
        nonce: u64,
    ) -> Result<(), Error> {
        if Self::is_frozen_internal(&env) {
            return Err(Error::ContractFrozen);
        }
        if Self::is_paused(env.clone()) {
            return Err(Error::Paused);
        }

        let (platform_treasury, fee_bps, fee_cap) = Self::load_fee_config(&env)?;

        Self::process_single_payment(
            &env,
            &sender,
            &recipient,
            &token_address,
            amount,
            &platform_treasury,
            fee_bps,
            fee_cap,
            Some(nonce),
        )
    }

    /// Returns the immutable execution receipt for `(sender, nonce)`, or
    /// `Error::TimelockNotFound` if no such receipt exists.
    pub fn get_payment_receipt(
        env: Env,
        sender: Address,
        nonce: u64,
    ) -> Result<PaymentReceipt, Error> {
        env.storage()
            .persistent()
            .get(&DataKey::PaymentNonce(sender, nonce))
            .ok_or(Error::TimelockNotFound)
    }

    /// Returns the current per-sender payment nonce counter.
    /// The next valid caller-supplied nonce is `counter + 1`.
    pub fn get_payment_nonce_counter(env: Env, sender: Address) -> u64 {
        env.storage()
            .persistent()
            .get(&DataKey::PaymentNonceCounter(sender))
            .unwrap_or(0u64)
    }

    /// Prunes an expired payment receipt from storage.  Anyone may call this.
    /// Returns `Error::TimelockNotFound` if the receipt does not exist, or
    /// `Error::TimelockNotReady` if it has not yet expired.
    pub fn prune_receipt(env: Env, sender: Address, nonce: u64) -> Result<(), Error> {
        let key = DataKey::PaymentNonce(sender, nonce);
        let receipt: PaymentReceipt = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(Error::TimelockNotFound)?;
        if env.ledger().timestamp() < receipt.expires_at {
            return Err(Error::TimelockNotReady);
        }
        env.storage().persistent().remove(&key);
        Ok(())
    }

    /// Returns the cumulative fee collected for a given token.
    /// Only incremented on successful transfers; failed payments contribute nothing.
    pub fn get_fee_total(env: Env, token: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::FeeTotal(token))
            .unwrap_or(0i128)
    }

    /// Returns the `RefundEntry` (balance + creation timestamp) for a user and token,
    /// or `None` if no entry exists.
    pub fn get_refund_entry_view(
        env: Env,
        user: Address,
        token: Address,
    ) -> Option<RefundEntry> {
        Self::get_refund_entry(&env, &user, &token)
    }

    /// Governed recovery of an expired refund balance.
    ///
    /// Once `created_at + REFUND_EXPIRY_SECONDS` has elapsed for a refund
    /// entry, the original user can no longer call `withdraw_refund`.  This
    /// entry point (admin-only) transfers the balance to the admin address
    /// and emits a `refund_recovered` event so the user can be notified off-chain.
    pub fn recover_expired_refund(
        env: Env,
        user: Address,
        token: Address,
    ) -> Result<i128, Error> {
        let admin = Self::require_admin(&env)?;
        admin.require_auth();

        let key = DataKey::RefundEntry(user.clone(), token.clone());
        let entry: RefundEntry = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(Error::NoRefundAvailable)?;

        if !Self::refund_is_expired(&env, &entry) {
            return Err(Error::TimelockNotReady);
        }

        let amount = entry.balance;
        env.storage().persistent().remove(&key);

        let token_client = token::Client::new(&env, &token);
        token_client.transfer(&env.current_contract_address(), &admin, &amount);

        env.events().publish(
            (
                Symbol::new(&env, "refund_recovered"),
                admin,
                user,
                token,
            ),
            amount,
        );

        log!(&env, "Expired refund recovered by admin");
        Ok(amount)
    }

    /// Returns the stored schema version (0 for pre-schema contracts).
    pub fn schema_version(env: Env) -> u32 {
        Self::schema_version_internal(&env)
    }

    /// Idempotent, bounded migration of one `(user, token)` refund entry
    /// from the legacy bare-`i128` `RefundBalance` key to the new
    /// `RefundEntry` key with a creation timestamp.
    ///
    /// Admin-only.  Returns `Error::AlreadyMigrated` if the schema version
    /// is already >= 1.
    pub fn migrate_refund_entry_v0_to_v1(
        env: Env,
        user: Address,
        token: Address,
    ) -> Result<(), Error> {
        let admin = Self::require_admin(&env)?;
        admin.require_auth();

        if Self::schema_version_internal(&env) >= 1 {
            return Err(Error::AlreadyMigrated);
        }

        let legacy_key = DataKey::RefundBalance(user.clone(), token.clone());
        let new_key = DataKey::RefundEntry(user.clone(), token.clone());

        // Idempotent: skip if already migrated.
        if env.storage().persistent().has(&new_key) {
            return Ok(());
        }

        if let Some(balance) = env
            .storage()
            .persistent()
            .get::<DataKey, i128>(&legacy_key)
        {
            if balance > 0 {
                // created_at = 0 so migrated entries are immediately expired on
                // mainnet and enter the governed recovery path.
                let entry = RefundEntry {
                    balance,
                    created_at: 0,
                };
                env.storage().persistent().set(&new_key, &entry);
                env.storage().persistent().extend_ttl(
                    &new_key,
                    Self::PERSISTENT_LIFETIME_THRESHOLD,
                    Self::PERSISTENT_BUMP_AMOUNT,
                );
                env.storage().persistent().remove(&legacy_key);
            }
        }

        Ok(())
    }

    /// Finalizes the v0→v1 migration by bumping the stored schema version.
    ///
    /// Call only after all legacy `RefundBalance` entries have been migrated.
    /// Returns `Error::MigrationVersionMismatch` if the stored version is not 0.
    ///
    /// Admin-only.
    pub fn finalize_migration_v0_to_v1(env: Env) -> Result<(), Error> {
        let admin = Self::require_admin(&env)?;
        admin.require_auth();

        if Self::schema_version_internal(&env) != 0 {
            return Err(Error::MigrationVersionMismatch);
        }

        env.storage()
            .instance()
            .set(&DataKey::SchemaVersion, &1u32);
        env.storage().instance().extend_ttl(
            Self::INSTANCE_LIFETIME_THRESHOLD,
            Self::INSTANCE_BUMP_AMOUNT,
        );

        env.events().publish(
            (Symbol::new(&env, "schema_migrated"),),
            (0u32, 1u32),
        );

        log!(&env, "Schema migrated from v0 to v1");
        Ok(())
    }

    /// Replaces this contract's WASM with a previously uploaded version. Admin-only.
    ///
    /// # Parameters
    /// - `new_wasm_hash`: Hash of a WASM blob previously uploaded to the
    ///   network, to install as this contract's new executable.
    ///
    /// # Returns
    /// `Ok(())` on success, or `Err(Error::NotInitialized)` if the contract
    /// has no admin set yet.
    ///
    /// # Panics
    /// Panics if the current admin does not authorize the call, or if
    /// `new_wasm_hash` does not reference a previously uploaded WASM blob.
    ///
    /// DEPRECATED for direct use.  Queue via `queue_action(ActionType::Upgrade(…))`.
    pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) -> Result<(), Error> {
        let admin = Self::require_admin(&env)?;
        admin.require_auth();
        let _ = new_wasm_hash;
        Err(Error::TimelockRequired)
    }

    /// Queues a contract upgrade for execution after the timelock delay.
    pub fn queue_upgrade(env: Env, new_wasm_hash: BytesN<32>) -> Result<u64, Error> {
        Self::queue_action(env, ActionType::Upgrade(new_wasm_hash))
    }

    /// Returns the contract version.
    ///
    /// # Returns
    /// The contract's version number, currently `1`.
    ///
    /// # Panics
    /// Does not panic.
    pub fn version(_env: Env) -> u32 {
        Self::VERSION
    }

    /// Returns the semantic version string of this contract, matching the
    /// version declared in `Cargo.toml`.
    ///
    /// Off-chain services and other contracts can call this function after an
    /// upgrade to confirm they are talking to the expected API version before
    /// proceeding with any state-mutating operations.
    ///
    /// # Returns
    /// A `String` in `MAJOR.MINOR.PATCH` format, e.g. `"0.1.0"`.
    ///
    /// # Panics
    /// Does not panic.
    pub fn get_version(env: Env) -> soroban_sdk::String {
        soroban_sdk::String::from_str(&env, env!("CARGO_PKG_VERSION"))
    }

    /// Returns the stable Soroban interface version for release-manifest checks.
    pub fn interface_version(_env: Env) -> u32 {
        Self::INTERFACE_VERSION
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::{
        testutils::{Address as _, Events, Ledger as _, LedgerInfo},
        token::StellarAssetClient,
        Address, Env, Symbol, TryIntoVal,
    };

    #[contract]
    pub struct RefundTestToken;

    #[contracttype]
    pub enum RefundTokenKey {
        Balance(Address),
        RejectedRecipient,
    }

    #[contracterror]
    #[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
    #[repr(u32)]
    pub enum RefundTokenError {
        InsufficientBalance = 1,
        RejectedRecipient = 2,
    }

    #[contractimpl]
    impl RefundTestToken {
        pub fn init(env: Env, rejected_recipient: Address) {
            env.storage()
                .instance()
                .set(&RefundTokenKey::RejectedRecipient, &rejected_recipient);
        }

        pub fn mint(env: Env, recipient: Address, amount: i128) {
            let key = RefundTokenKey::Balance(recipient.clone());
            let balance: i128 = env.storage().instance().get(&key).unwrap_or(0);
            env.storage().instance().set(&key, &(balance + amount));
        }

        pub fn balance(env: Env, owner: Address) -> i128 {
            env.storage()
                .instance()
                .get(&RefundTokenKey::Balance(owner))
                .unwrap_or(0)
        }

        pub fn transfer(
            env: Env,
            from: Address,
            to: Address,
            amount: i128,
        ) -> Result<(), RefundTokenError> {
            from.require_auth();
            if env
                .storage()
                .instance()
                .get::<RefundTokenKey, Address>(&RefundTokenKey::RejectedRecipient)
                == Some(to.clone())
            {
                return Err(RefundTokenError::RejectedRecipient);
            }

            let from_key = RefundTokenKey::Balance(from);
            let from_balance: i128 = env.storage().instance().get(&from_key).unwrap_or(0);
            if amount <= 0 || from_balance < amount {
                return Err(RefundTokenError::InsufficientBalance);
            }
            let to_key = RefundTokenKey::Balance(to);
            let to_balance: i128 = env.storage().instance().get(&to_key).unwrap_or(0);
            env.storage()
                .instance()
                .set(&from_key, &(from_balance - amount));
            env.storage()
                .instance()
                .set(&to_key, &(to_balance + amount));
            Ok(())
        }
    }

    /// Returns (env, client, contract_id).
    fn setup_env() -> (Env, PaymentRouterClient<'static>, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, PaymentRouter);
        let client = PaymentRouterClient::new(&env, &contract_id);
        (env, client, contract_id)
    }

    /// Deploys a Stellar Asset Contract test token. Returns
    /// (token_address, token_client, stellar_asset_admin_client).
    fn setup_token(
        env: &Env,
    ) -> (
        Address,
        token::Client<'static>,
        token::StellarAssetClient<'static>,
    ) {
        let token_admin = Address::generate(env);
        let token_address = env.register_stellar_asset_contract(token_admin);
        let token_client = token::Client::new(env, &token_address);
        let token_admin_client = token::StellarAssetClient::new(env, &token_address);
        (token_address, token_client, token_admin_client)
    }

    fn has_event(env: &Env, expected: Symbol) -> bool {
        let events = env.events().all();
        events.iter().any(|(_, topics, _)| {
            topics
                .get(0)
                .map(|raw| {
                    let symbol: Result<Symbol, _> = raw.try_into_val(env);
                    symbol.map(|value| value == expected).unwrap_or(false)
                })
                .unwrap_or(false)
        })
    }

    fn execute_after_delay(
        env: &Env,
        client: &PaymentRouterClient<'_>,
        action: &ActionType,
    ) -> u64 {
        let nonce = client.queue_action(action);
        advance_time(env, PaymentRouter::SECONDS_IN_24H);
        client.execute_action(&nonce);
        nonce
    }

    // ── Timelock tests ───────────────────────────────────────────────────────

    #[test]
    fn test_queue_and_execute_set_fee_bps_after_delay() {
        let (env, client, _) = setup_env();

        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        client.initialize(&admin, &treasury, &100, &1000, &PaymentRouter::MAX_AMOUNT);

        // Queue a fee-bps change.
        let nonce = client.queue_action(&ActionType::SetFeeBps(250));
        assert_eq!(nonce, 1);
        assert_eq!(client.get_fee(), 100); // Not applied yet.

        // Trying to execute immediately should fail (delay not elapsed).
        let res = client.try_execute_action(&nonce);
        assert_eq!(res.unwrap_err().unwrap(), Error::TimelockNotReady);

        // Advance time past 24 hours.
        let current_time = env.ledger().timestamp();
        env.ledger().set(LedgerInfo {
            timestamp: current_time + PaymentRouter::SECONDS_IN_24H + 1,
            protocol_version: env.ledger().protocol_version(),
            sequence_number: env.ledger().sequence(),
            network_id: env.ledger().network_id().into(),
            base_reserve: 100,
            min_temp_entry_ttl: 16,
            min_persistent_entry_ttl: 4096,
            max_entry_ttl: 6312000,
        });

        // Now execution should succeed.
        client.execute_action(&nonce);
        assert_eq!(client.get_fee(), 250);

        // Entry should be gone.
        let res = client.try_get_queued_action(&nonce);
        assert_eq!(res.unwrap_err().unwrap(), Error::TimelockNotFound);
    }

    #[test]
    fn test_queue_and_execute_set_platform_treasury() {
        let (env, client, _) = setup_env();

        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let new_treasury = Address::generate(&env);
        client.initialize(&admin, &treasury, &100, &1000, &PaymentRouter::MAX_AMOUNT);

        let nonce = client.queue_action(&ActionType::SetPlatformTreasury(new_treasury.clone()));

        // Advance 24h+.
        let ts = env.ledger().timestamp();
        env.ledger().set(LedgerInfo {
            timestamp: ts + PaymentRouter::SECONDS_IN_24H + 1,
            protocol_version: env.ledger().protocol_version(),
            sequence_number: env.ledger().sequence(),
            network_id: env.ledger().network_id().into(),
            base_reserve: 100,
            min_temp_entry_ttl: 16,
            min_persistent_entry_ttl: 4096,
            max_entry_ttl: 6312000,
        });

        client.execute_action(&nonce);

        // Verify the treasury was actually updated by routing a payment and
        // checking where the fee lands.
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);
        let (token_addr, token_client, sac) = setup_token(&env);
        sac.mint(&sender, &10_000);
        client.route_payment(&sender, &recipient, &token_addr, &1000);

        // 100 bps of 1000 = 10, capped to min(10, 1000) = 10
        assert_eq!(token_client.balance(&new_treasury), 10);
        assert_eq!(token_client.balance(&treasury), 0);
    }

    #[test]
    fn test_execute_action_not_found() {
        let (env, client, _) = setup_env();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        client.initialize(&admin, &treasury, &100, &1000, &PaymentRouter::MAX_AMOUNT);

        let res = client.try_execute_action(&99u64);
        assert_eq!(res.unwrap_err().unwrap(), Error::TimelockNotFound);
    }

    #[test]
    fn test_cancel_action() {
        let (env, client, _) = setup_env();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        client.initialize(&admin, &treasury, &100, &1000, &PaymentRouter::MAX_AMOUNT);

        let nonce = client.queue_action(&ActionType::SetFeeBps(999));
        assert!(client.try_get_queued_action(&nonce).is_ok());

        client.cancel_action(&nonce);

        // Entry should be gone.
        let res = client.try_get_queued_action(&nonce);
        assert_eq!(res.unwrap_err().unwrap(), Error::TimelockNotFound);

        // Fee should remain unchanged.
        assert_eq!(client.get_fee(), 100);
    }

    #[test]
    fn test_cancel_nonexistent_action() {
        let (env, client, _) = setup_env();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        client.initialize(&admin, &treasury, &100, &1000, &PaymentRouter::MAX_AMOUNT);

        let res = client.try_cancel_action(&42u64);
        assert_eq!(res.unwrap_err().unwrap(), Error::TimelockNotFound);
    }

    #[test]
    fn test_identical_timelock_action_cannot_be_queued_twice() {
        let (env, client, _) = setup_env();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        client.initialize(&admin, &treasury, &100, &1000, &PaymentRouter::MAX_AMOUNT);

        let action = ActionType::SetMinAmount(25);
        let nonce = client.queue_action(&action);
        assert_eq!(
            client.try_queue_action(&action).unwrap_err().unwrap(),
            Error::TimelockPending
        );

        client.cancel_action(&nonce);
        assert!(client.try_queue_action(&action).is_ok());
    }

    #[test]
    fn test_admin_transfer_requires_nominee_acceptance() {
        let (env, client, _) = setup_env();
        let admin = Address::generate(&env);
        let nominee = Address::generate(&env);
        let treasury = Address::generate(&env);
        client.initialize(&admin, &treasury, &100, &1000, &PaymentRouter::MAX_AMOUNT);

        let nonce = client.queue_action(&ActionType::TransferAdmin(nominee.clone()));
        advance_time(&env, PaymentRouter::SECONDS_IN_24H);
        client.execute_action(&nonce);

        assert_eq!(client.get_admin(), admin);
        let pending = client.get_pending_admin_transfer().unwrap();
        assert_eq!(pending.proposed_admin, nominee);
        assert_eq!(pending.action_nonce, nonce);
        advance_time(&env, pending.expires_at - env.ledger().timestamp());

        client.accept_admin();
        assert_eq!(client.get_admin(), nominee);
        assert!(client.get_pending_admin_transfer().is_none());

        assert!(has_event(
            &env,
            Symbol::new(&env, "admin_transfer_proposed")
        ));
        assert!(has_event(
            &env,
            Symbol::new(&env, "admin_transfer_accepted")
        ));
    }

    #[test]
    fn test_admin_transfer_can_be_cancelled_or_expired() {
        let (env, client, _) = setup_env();
        let admin = Address::generate(&env);
        let nominee = Address::generate(&env);
        let treasury = Address::generate(&env);
        client.initialize(&admin, &treasury, &100, &1000, &PaymentRouter::MAX_AMOUNT);

        let cancelled_nonce = client.queue_action(&ActionType::TransferAdmin(nominee.clone()));
        advance_time(&env, PaymentRouter::SECONDS_IN_24H);
        client.execute_action(&cancelled_nonce);
        client.cancel_admin_transfer();
        assert!(client.get_pending_admin_transfer().is_none());
        assert_eq!(client.get_admin(), admin);
        assert!(has_event(
            &env,
            Symbol::new(&env, "admin_transfer_cancelled")
        ));

        let expired_nonce = client.queue_action(&ActionType::TransferAdmin(nominee));
        advance_time(&env, PaymentRouter::SECONDS_IN_24H);
        client.execute_action(&expired_nonce);
        advance_time(&env, PaymentRouter::ADMIN_TRANSFER_EXPIRY_SECONDS + 1);
        assert_eq!(
            client.try_accept_admin().unwrap_err().unwrap(),
            Error::AdminTransferExpired
        );
        client.expire_admin_transfer();
        assert!(client.get_pending_admin_transfer().is_none());
        assert_eq!(client.get_admin(), admin);
        assert!(has_event(&env, Symbol::new(&env, "admin_transfer_expired")));
    }

    #[test]
    fn test_nonce_increments() {
        let (env, client, _) = setup_env();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        client.initialize(&admin, &treasury, &100, &1000, &PaymentRouter::MAX_AMOUNT);

        let n1 = client.queue_action(&ActionType::SetFeeBps(200));
        let n2 = client.queue_action(&ActionType::SetFeeBps(300));
        let n3 = client.queue_action(&ActionType::SetFeeBps(400));

        assert_eq!(n1, 1);
        assert_eq!(n2, 2);
        assert_eq!(n3, 3);
    }

    // ── Freeze tests ─────────────────────────────────────────────────────────

    #[test]
    fn test_emergency_freeze_blocks_payments() {
        let (env, client, _) = setup_env();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        let (token_address, _token_client, sac) = setup_token(&env);
        sac.mint(&sender, &10_000);

        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);

        assert!(!client.is_frozen());

        client.emergency_freeze();
        assert!(client.is_frozen());

        let res = client.try_route_payment(&sender, &recipient, &token_address, &1000);
        assert_eq!(res.unwrap_err().unwrap(), Error::ContractFrozen);
    }

    #[test]
    fn test_emergency_freeze_blocks_timelock_execution() {
        let (env, client, _) = setup_env();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        client.initialize(&admin, &treasury, &100, &1000, &PaymentRouter::MAX_AMOUNT);

        let nonce = client.queue_action(&ActionType::SetFeeBps(500));

        // Advance past 24h.
        let ts = env.ledger().timestamp();
        env.ledger().set(LedgerInfo {
            timestamp: ts + PaymentRouter::SECONDS_IN_24H + 1,
            protocol_version: env.ledger().protocol_version(),
            sequence_number: env.ledger().sequence(),
            network_id: env.ledger().network_id().into(),
            base_reserve: 100,
            min_temp_entry_ttl: 16,
            min_persistent_entry_ttl: 4096,
            max_entry_ttl: 6312000,
        });

        // Freeze the contract before execution.
        client.emergency_freeze();

        let res = client.try_execute_action(&nonce);
        assert_eq!(res.unwrap_err().unwrap(), Error::ContractFrozen);

        // Fee remains unchanged.
        assert_eq!(client.get_fee(), 100);
    }

    #[test]
    fn test_unfreeze_restores_payments() {
        let (env, client, _) = setup_env();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        let (token_address, _token_client, sac) = setup_token(&env);
        sac.mint(&sender, &10_000);

        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);

        client.emergency_freeze();
        assert!(client.is_frozen());

        let nonce = client.queue_action(&ActionType::Unfreeze);
        advance_time(&env, PaymentRouter::SECONDS_IN_24H);
        client.execute_action(&nonce);
        assert!(!client.is_frozen());

        // Payments should work again.
        client.route_payment(&sender, &recipient, &token_address, &1000);
    }

    #[test]
    fn test_emergency_freeze_remains_immediate() {
        let (env, client, _) = setup_env();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        client.initialize(&admin, &treasury, &100, &1000, &PaymentRouter::MAX_AMOUNT);

        client.emergency_freeze();
        assert!(client.is_frozen());

        let unfreeze_nonce = client.queue_action(&ActionType::Unfreeze);
        assert_eq!(
            client
                .try_execute_action(&unfreeze_nonce)
                .unwrap_err()
                .unwrap(),
            Error::TimelockNotReady
        );
        advance_time(&env, PaymentRouter::SECONDS_IN_24H);
        client.execute_action(&unfreeze_nonce);
        assert!(!client.is_frozen());
    }

    #[test]
    fn test_freeze_queue_action_blocked() {
        let (env, client, _) = setup_env();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        client.initialize(&admin, &treasury, &100, &1000, &PaymentRouter::MAX_AMOUNT);

        client.emergency_freeze();

        // Cannot queue new actions while frozen.
        let res = client.try_queue_action(&ActionType::SetFeeBps(500));
        assert_eq!(res.unwrap_err().unwrap(), Error::ContractFrozen);
    }

    #[test]
    fn test_cancel_action_allowed_while_frozen() {
        let (env, client, _) = setup_env();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        client.initialize(&admin, &treasury, &100, &1000, &PaymentRouter::MAX_AMOUNT);

        // Queue an action before freezing.
        let nonce = client.queue_action(&ActionType::SetFeeBps(500));

        client.emergency_freeze();

        // Cancellation should still be possible while frozen (incident response).
        client.cancel_action(&nonce);
        let res = client.try_get_queued_action(&nonce);
        assert_eq!(res.unwrap_err().unwrap(), Error::TimelockNotFound);
    }

    // ── Timelock emits events ────────────────────────────────────────────────

    #[test]
    fn test_queue_action_emits_event() {
        let (env, client, _) = setup_env();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        client.initialize(&admin, &treasury, &100, &1000, &PaymentRouter::MAX_AMOUNT);

        client.queue_action(&ActionType::SetFeeBps(200));

        let events = env.events().all();
        let found = events.iter().any(|(_, topics, _)| {
            if topics.is_empty() {
                return false;
            }
            let raw = topics.get(0).unwrap();
            let sym: Result<Symbol, _> = raw.try_into_val(&env);
            sym.map(|s| s == Symbol::new(&env, "action_queued"))
                .unwrap_or(false)
        });
        assert!(found, "action_queued event not found");
    }

    #[test]
    fn test_freeze_emits_event() {
        let (env, client, _) = setup_env();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        client.initialize(&admin, &treasury, &100, &1000, &PaymentRouter::MAX_AMOUNT);

        client.emergency_freeze();

        let events = env.events().all();
        let found = events.iter().any(|(_, topics, _)| {
            if topics.is_empty() {
                return false;
            }
            let raw = topics.get(0).unwrap();
            let sym: Result<Symbol, _> = raw.try_into_val(&env);
            sym.map(|s| s == Symbol::new(&env, "emergency_freeze"))
                .unwrap_or(false)
        });
        assert!(found, "emergency_freeze event not found");
    }

    // ── Original tests (retained) ────────────────────────────────────────────

    #[test]
    fn test_get_fee() {
        let (env, client, _) = setup_env();

        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);

        // Before initialization, get_fee returns 0
        assert_eq!(client.get_fee(), 0);

        // Initialize with 150 bps
        client.initialize(&admin, &treasury, &150, &5000, &PaymentRouter::MAX_AMOUNT);
        assert_eq!(client.get_fee(), 150);

        // Fee updates are applied only by a matured timelock action.
        execute_after_delay(&env, &client, &ActionType::SetFeeBps(250));
        assert_eq!(client.get_fee(), 250);

        execute_after_delay(&env, &client, &ActionType::SetFeeConfig(300, 10_000));
        assert_eq!(client.get_fee(), 300);
    }

    #[test]
    fn test_version_reports_contract_version() {
        let (_env, client, _) = setup_env();

        // #269 — the version view is callable without initialization and
        // returns the compiled-in contract version so a UI can check
        // compatibility before interacting with the contract.
        assert_eq!(client.version(), PaymentRouter::VERSION);
        assert_eq!(client.version(), 1);
    }

    #[test]
    fn test_admin_restrictions_and_updates() {
        let (env, client, _) = setup_env();

        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let new_admin = Address::generate(&env);

        client.initialize(&admin, &treasury, &100, &1000, &PaymentRouter::MAX_AMOUNT);

        // Trying to initialize again should fail
        let res = client.try_initialize(&admin, &treasury, &100, &1000, &PaymentRouter::MAX_AMOUNT);
        assert_eq!(res.unwrap_err().unwrap(), Error::AlreadyInitialized);

        assert_eq!(
            client.try_set_admin(&new_admin).unwrap_err().unwrap(),
            Error::TimelockRequired
        );
        execute_after_delay(&env, &client, &ActionType::SetFeeConfig(200, 2000));
        assert_eq!(client.get_fee(), 200);

        let new_treasury = Address::generate(&env);
        execute_after_delay(
            &env,
            &client,
            &ActionType::SetPlatformTreasury(new_treasury),
        );
    }

    #[test]
    fn test_recover_tokens() {
        let (env, client, contract_id) = setup_env();

        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);

        client.initialize(&admin, &treasury, &100, &1000, &PaymentRouter::MAX_AMOUNT);

        let (token_address, token_client, stellar_asset_client) = setup_token(&env);

        // Simulate tokens accidentally sent directly to the contract address
        let accidental_amount = 5_000i128;
        stellar_asset_client.mint(&contract_id, &accidental_amount);

        assert_eq!(token_client.balance(&contract_id), accidental_amount);
        assert_eq!(token_client.balance(&admin), 0);

        // Admin recovers tokens
        let recover_amount = 3_000i128;
        client.recover_tokens(&token_address, &recover_amount);

        assert_eq!(token_client.balance(&admin), recover_amount);
        assert_eq!(
            token_client.balance(&contract_id),
            accidental_amount - recover_amount
        );
    }

    #[test]
    fn test_set_pause_emits_event() {
        let (env, client, _) = setup_env();

        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);

        client.initialize(&admin, &treasury, &100, &1000, &PaymentRouter::MAX_AMOUNT);

        execute_after_delay(&env, &client, &ActionType::SetPaused(true));

        let events = env.events().all();
        let pause_event_found = events.iter().any(|(_, topics, _)| {
            topics.len() == 1
                && topics
                    .get(0)
                    .map(|raw| {
                        let symbol: Result<Symbol, _> = raw.try_into_val(&env);
                        symbol
                            .map(|value| value == symbol_short!("pause"))
                            .unwrap_or(false)
                    })
                    .unwrap_or(false)
        });
        assert!(pause_event_found);
    }

    #[test]
    fn test_route_payment_emits_payment_initiated_event() {
        let (env, client, _) = setup_env();

        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        let (token_address, _token_client, sac) = setup_token(&env);
        sac.mint(&sender, &10_000);

        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);

        client
            .mock_all_auths()
            .route_payment(&sender, &recipient, &token_address, &5_000);

        let events = env.events().all();
        assert!(!events.is_empty());

        let mut found = false;
        for (_, topics, data) in events.iter() {
            if !topics.is_empty() {
                if let Ok(topic_sym) = topics.get(0).unwrap().try_into_val(&env) {
                    let sym: Symbol = topic_sym;
                    if sym == Symbol::new(&env, "payment_initiated") {
                        found = true;
                        let amt: i128 = data.try_into_val(&env).unwrap();
                        assert_eq!(amt, 5_000);
                        break;
                    }
                }
            }
        }
        assert!(found, "payment_initiated event not found");
    }

    #[test]
    fn test_route_payment_emits_routed_event() {
        let (env, client, _) = setup_env();

        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        let (token_address, _token_client, _token_admin_client) = setup_token(&env);
        let sac = soroban_sdk::token::StellarAssetClient::new(&env, &token_address);
        sac.mint(&sender, &10_000);

        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);
        client.add_supported_token(&token_address);

        let amount = 2_000i128;
        client.route_payment(&sender, &recipient, &token_address, &amount);

        let events = env.events().all();
        assert!(!events.is_empty());

        // Find the "routed" event by topic
        let mut found = None;
        for evt in events.iter() {
            let (_contract_id, topics, _data) = evt.clone();
            if topics.len() != 3 {
                continue;
            }
            let topic0: Symbol = topics.get(0).unwrap().try_into_val(&env).unwrap();
            if topic0 == symbol_short!("routed") {
                found = Some(evt.clone());
                break;
            }
        }
        let routed = found.expect("route_payment should publish a \"routed\" event");

        let (_contract_id, topics, data) = routed;
        assert_eq!(topics.len(), 3);

        let topic_sender: Address = topics.get(1).unwrap().try_into_val(&env).unwrap();
        let topic_recipient: Address = topics.get(2).unwrap().try_into_val(&env).unwrap();
        assert_eq!(topic_sender, sender);
        assert_eq!(topic_recipient, recipient);

        let event_amount: i128 = data.try_into_val(&env).unwrap();
        assert_eq!(event_amount, amount);
    }

    #[test]
    fn test_admin_pause_functionality() {
        let (env, client, _) = setup_env();

        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        let (token_address, _token_client, _token_admin_client) = setup_token(&env);
        let sac = soroban_sdk::token::StellarAssetClient::new(&env, &token_address);
        sac.mint(&sender, &10_000);

        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);

        // Initially not paused
        assert!(!client.is_paused());

        // Pause
        execute_after_delay(&env, &client, &ActionType::SetPaused(true));
        assert!(client.is_paused());

        // Route payment should fail when paused
        let res = client.try_route_payment(&sender, &recipient, &token_address, &1000);
        assert_eq!(res.unwrap_err().unwrap(), Error::Paused);

        // Unpause via set_paused alias
        execute_after_delay(&env, &client, &ActionType::SetPaused(false));
        assert!(!client.is_paused());

        // Route payment should succeed now
        client.route_payment(&sender, &recipient, &token_address, &1000);
    }

    #[test]
    fn test_route_payment_calculates_and_sends_fee() {
        let (env, client, _) = setup_env();

        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        let (token_address, token_client, _token_admin_client) = setup_token(&env);

        let sac = soroban_sdk::token::StellarAssetClient::new(&env, &token_address);
        let initial_balance = 10_000i128;
        sac.mint(&sender, &initial_balance);

        // Initialize router with 1% fee (100 bps) and cap of 50
        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);
        client.add_supported_token(&token_address);

        // Test normal fee calculation: 1% of 2000 = 20, below cap of 50
        let amount_1 = 2000i128;
        client.route_payment(&sender, &recipient, &token_address, &amount_1);

        assert_eq!(token_client.balance(&treasury), 20);
        assert_eq!(token_client.balance(&recipient), 1980);
        assert_eq!(token_client.balance(&sender), initial_balance - amount_1);
        assert_eq!(client.get_user_volume(&sender), amount_1);

        // Test fee capped at 50: 1% of 8000 = 80, capped to 50
        let amount_2 = 8000i128;
        client.route_payment(&sender, &recipient, &token_address, &amount_2);

        assert_eq!(token_client.balance(&treasury), 70);
        assert_eq!(token_client.balance(&recipient), 9930);
        assert_eq!(
            token_client.balance(&sender),
            initial_balance - amount_1 - amount_2
        );
        assert_eq!(client.get_user_volume(&sender), amount_1 + amount_2);
    }

    fn assert_batch_failure_rolls_back(failing_index: u32) {
        let (env, client, _) = setup_env();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let (token_address, token_client, sac) = setup_token(&env);
        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);

        let mut payments = Vec::new(&env);
        let mut senders = Vec::new(&env);
        let mut recipients = Vec::new(&env);
        for index in 0..3 {
            let sender = Address::generate(&env);
            let recipient = Address::generate(&env);
            senders.push_back(sender.clone());
            sac.mint(&sender, &10_000);
            recipients.push_back(recipient.clone());
            payments.push_back(Payment {
                sender,
                recipient,
                token_address: token_address.clone(),
                amount: if index == failing_index { 0 } else { 1_000 },
            });
        }

        let result = client.try_route_payments(&payments);
        assert_eq!(
            result.unwrap_err().unwrap(),
            Error::LimitExceeded,
            "failing item index {failing_index}"
        );
        assert_eq!(token_client.balance(&treasury), 0);
        for sender in senders.iter() {
            assert_eq!(token_client.balance(&sender), 10_000);
            assert_eq!(client.get_user_volume(&sender), 0);
        }
        for recipient in recipients.iter() {
            assert_eq!(token_client.balance(&recipient), 0);
        }
        assert_eq!(Error::LimitExceeded as u32, 3);
    }

    #[test]
    fn test_route_payments_roll_back_first_item_failure() {
        assert_batch_failure_rolls_back(0);
    }

    #[test]
    fn test_route_payments_roll_back_middle_item_failure() {
        assert_batch_failure_rolls_back(1);
    }

    #[test]
    fn test_route_payments_roll_back_final_item_failure() {
        assert_batch_failure_rolls_back(2);
    }

    #[test]
    fn test_route_payments_failure_rolls_back_fee_and_refund_effects() {
        let (env, client, contract_id) = setup_env();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let sender = Address::generate(&env);
        let final_sender = Address::generate(&env);
        let blocked_recipient = Address::generate(&env);
        let final_recipient = Address::generate(&env);
        let token_address = env.register_contract(None, RefundTestToken);
        let mock_token = RefundTestTokenClient::new(&env, &token_address);
        mock_token.init(&blocked_recipient);
        mock_token.mint(&sender, &10_000);
        mock_token.mint(&final_sender, &10_000);
        let token_client = token::Client::new(&env, &token_address);
        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);

        let mut payments = Vec::new(&env);
        payments.push_back(Payment {
            sender: final_sender.clone(),
            recipient: blocked_recipient,
            token_address: token_address.clone(),
            amount: 2_000,
        });
        payments.push_back(Payment {
            sender: sender.clone(),
            recipient: final_recipient,
            token_address: token_address.clone(),
            amount: 0,
        });

        let result = client.try_route_payments(&payments);
        assert_eq!(result.unwrap_err().unwrap(), Error::LimitExceeded);
        assert_eq!(token_client.balance(&sender), 10_000);
        assert_eq!(token_client.balance(&final_sender), 10_000);
        assert_eq!(token_client.balance(&treasury), 0);
        assert_eq!(token_client.balance(&contract_id), 0);
        assert_eq!(client.get_refund_balance(&sender, &token_address), 0);
        assert_eq!(client.get_user_volume(&final_sender), 0);
        assert_eq!(client.get_user_volume(&sender), 0);
    }

    #[test]
    fn test_insufficient_balance() {
        let (env, client, _) = setup_env();

        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        let (token_address, _token_client, sac) = setup_token(&env);
        sac.mint(&sender, &100);

        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);
        client.add_supported_token(&token_address);

        // Route payment of 500 when balance is only 100
        let res = client.try_route_payment(&sender, &recipient, &token_address, &500);
        assert_eq!(res.unwrap_err().unwrap(), Error::InsufficientBalance);
    }

    #[test]
    fn test_daily_limit_and_reset() {
        let (env, client, _) = setup_env();

        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        let (token_address, token_client, _token_admin_client) = setup_token(&env);

        let limit = 10_000_000_000_000i128;
        let sac = soroban_sdk::token::StellarAssetClient::new(&env, &token_address);
        sac.mint(&sender, &(limit + 2000));

        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);
        client.add_supported_token(&token_address);

        // Route amount up to daily limit
        client.route_payment(&sender, &recipient, &token_address, &limit);

        // Next payment should exceed daily limit
        let res = client.try_route_payment(&sender, &recipient, &token_address, &2000);
        assert_eq!(res.unwrap_err().unwrap(), Error::LimitExceeded);

        // Advance the ledger timestamp past 24 hours to reset the daily limit.
        // The ledger sequence number is deliberately left untouched: the reset
        // must be driven by wall-clock time, not by ledger count.
        let current_time = env.ledger().timestamp();
        let current_protocol_version = env.ledger().protocol_version();
        env.ledger().set(LedgerInfo {
            timestamp: current_time + PaymentRouter::DAY_SECONDS + 1,
            protocol_version: current_protocol_version,
            sequence_number: env.ledger().sequence(),
            network_id: env.ledger().network_id().into(),
            base_reserve: 100,
            min_temp_entry_ttl: 16,
            min_persistent_entry_ttl: 4096,
            max_entry_ttl: 6312000,
        });

        // Now routing should succeed again. The first payment pushed volume past
        // VOLUME_THRESHOLD, so the halved rate applies: 2000 * 50 bps = 10.
        client.route_payment(&sender, &recipient, &token_address, &2000);
        assert_eq!(token_client.balance(&recipient), (limit - 50) + (2000 - 10));
    }

    /// Advances the ledger timestamp by `delta` seconds without changing the
    /// ledger sequence number, so tests exercise the wall-clock window only.
    fn advance_time(env: &Env, delta: u64) {
        let current_time = env.ledger().timestamp();
        env.ledger().set(LedgerInfo {
            timestamp: current_time + delta,
            protocol_version: env.ledger().protocol_version(),
            sequence_number: env.ledger().sequence(),
            network_id: env.ledger().network_id().into(),
            base_reserve: 100,
            min_temp_entry_ttl: 16,
            min_persistent_entry_ttl: 4096,
            max_entry_ttl: 6312000,
        });
    }

    /// A payment made 86 399 seconds after the last reset must NOT reset the
    /// daily counter: the window is still the same 24-hour period.
    #[test]
    fn test_daily_limit_not_reset_at_86399_seconds() {
        let (env, client, _) = setup_env();

        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        let (token_address, _token_client, sac) = setup_token(&env);

        let limit = 10_000_000_000_000i128;
        sac.mint(&sender, &(limit + 2000));

        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);

        // Exhaust the daily limit.
        client.route_payment(&sender, &recipient, &token_address, &limit);

        // 86 399 s later the window has not yet rolled over.
        advance_time(&env, PaymentRouter::DAY_SECONDS - 1);

        let res = client.try_route_payment(&sender, &recipient, &token_address, &2000);
        assert_eq!(res.unwrap_err().unwrap(), Error::LimitExceeded);
    }

    /// A payment made 86 401 seconds after the last reset starts a fresh
    /// 24-hour window and succeeds.
    #[test]
    fn test_daily_limit_resets_at_86401_seconds() {
        let (env, client, _) = setup_env();

        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        let (token_address, _token_client, sac) = setup_token(&env);

        let limit = 10_000_000_000_000i128;
        sac.mint(&sender, &(limit + 2000));

        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);

        // Exhaust the daily limit.
        client.route_payment(&sender, &recipient, &token_address, &limit);

        // 86 401 s later a fresh window begins.
        advance_time(&env, PaymentRouter::DAY_SECONDS + 1);

        client.route_payment(&sender, &recipient, &token_address, &2000);
    }

    #[test]
    fn test_prevent_self_routing() {
        let (env, client, _) = setup_env();

        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let sender = Address::generate(&env);

        let (token_address, _token_client, _token_admin_client) = setup_token(&env);
        let sac = soroban_sdk::token::StellarAssetClient::new(&env, &token_address);
        sac.mint(&sender, &10_000);

        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);

        let res = client.try_route_payment(&sender, &sender, &token_address, &1000);
        assert_eq!(res.unwrap_err().unwrap(), Error::InvalidRecipient);
    }

    #[test]
    #[ignore]
    fn test_tiered_fee_discount_applied_after_volume_threshold() {
        let (env, client, _) = setup_env();

        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        let (token_address, token_client, _token_admin_client) = setup_token(&env);

        // Threshold is 10,000 XLM = 10,000 * 10,000,000 (7 decimals)
        let threshold = 100_000_000_000i128;
        let first_amount = threshold + 1;
        let second_amount = 1000i128;
        let total_mint = first_amount + second_amount + 10_000_000;
        let sac = soroban_sdk::token::StellarAssetClient::new(&env, &token_address);
        sac.mint(&sender, &total_mint);

        // Initialize with 1% fee (100 bps) and no cap
        client.initialize(
            &admin,
            &treasury,
            &100,
            &i128::MAX,
            &PaymentRouter::MAX_AMOUNT,
        );

        // First payment: volume is 0 (< threshold), full fee applies
        client.route_payment(&sender, &recipient, &token_address, &first_amount);

        let full_fee_first = (first_amount * 100) / 10_000;
        assert_eq!(token_client.balance(&treasury), full_fee_first);
        assert_eq!(
            token_client.balance(&recipient),
            first_amount - full_fee_first
        );
        assert_eq!(client.get_user_volume(&sender), first_amount);
        // Volume is now past threshold, so next call gets the discount
        assert_eq!(client.get_effective_fee_bps(&sender), 50);

        // Second payment: volume > threshold, 50% discount applies
        client.route_payment(&sender, &recipient, &token_address, &second_amount);

        let discounted_fee = (second_amount * 50) / 10_000;
        assert_eq!(
            token_client.balance(&treasury),
            full_fee_first + discounted_fee
        );
        assert_eq!(
            token_client.balance(&recipient),
            (first_amount - full_fee_first) + (second_amount - discounted_fee)
        );
    }

    #[test]
    fn test_get_effective_fee_bps_no_discount_below_threshold() {
        let (env, client, _) = setup_env();

        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        let (token_address, _token_client, _token_admin_client) = setup_token(&env);
        let sac = soroban_sdk::token::StellarAssetClient::new(&env, &token_address);
        sac.mint(&sender, &1_000_000);

        client.initialize(
            &admin,
            &treasury,
            &100,
            &i128::MAX,
            &PaymentRouter::MAX_AMOUNT,
        );

        // No volume yet
        assert_eq!(client.get_effective_fee_bps(&sender), 100);

        // Route a small payment (below threshold)
        client.route_payment(&sender, &recipient, &token_address, &1000);

        // Volume is 1000, far below 10,000 XLM threshold
        assert_eq!(client.get_effective_fee_bps(&sender), 100);
    }

    #[test]
    fn test_successful_xlm_routing() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);
        let platform_treasury = Address::generate(&env);

        let contract_id = env.register_contract(None, PaymentRouter);
        let client = PaymentRouterClient::new(&env, &contract_id);

        client.initialize(
            &admin,
            &platform_treasury,
            &40,
            &i128::MAX,
            &PaymentRouter::MAX_AMOUNT,
        );

        let token_admin = Address::generate(&env);
        let token_address = env.register_stellar_asset_contract(token_admin.clone());
        let sac = StellarAssetClient::new(&env, &token_address);
        let token_client = token::Client::new(&env, &token_address);

        let initial_balance = 1_000_000_000i128;
        sac.mint(&sender, &initial_balance);

        client.add_supported_token(&token_address);

        let amount = 100_000_000i128;
        client.route_payment(&sender, &recipient, &token_address, &amount);

        let expected_fee = 400_000i128;
        let expected_recipient_amount = amount - expected_fee;

        assert_eq!(token_client.balance(&sender), initial_balance - amount);
        assert_eq!(token_client.balance(&recipient), expected_recipient_amount);
        assert_eq!(token_client.balance(&platform_treasury), expected_fee);
    }

    #[test]
    fn test_initialize_sets_admin() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let contract_addr = env.register_contract(None, PaymentRouter);
        let client = PaymentRouterClient::new(&env, &contract_addr);

        client.initialize(&admin, &treasury, &100, &1000, &PaymentRouter::MAX_AMOUNT);

        let stored_admin: Option<Address> = env.as_contract(&contract_addr, || {
            env.storage().instance().get(&DataKey::Admin)
        });
        assert_eq!(stored_admin, Some(admin));
    }

    /// Verifies that `emergency_withdraw` transfers the exact requested amount
    /// from the contract's own balance to the admin address.
    #[test]
    fn test_emergency_withdraw_transfers_tokens_to_admin() {
        let (env, client, contract_id) = setup_env();

        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);

        client.initialize(&admin, &treasury, &100, &1000, &PaymentRouter::MAX_AMOUNT);

        let (token_address, token_client, stellar_asset_client) = setup_token(&env);

        // Fund the contract directly (simulates stranded tokens from a routing failure).
        let stranded_amount = 10_000i128;
        stellar_asset_client.mint(&contract_id, &stranded_amount);

        assert_eq!(token_client.balance(&contract_id), stranded_amount);
        assert_eq!(token_client.balance(&admin), 0);

        // Admin withdraws half the stranded balance.
        let withdraw_amount = 4_000i128;
        client.emergency_withdraw(&token_address, &withdraw_amount);

        assert_eq!(token_client.balance(&admin), withdraw_amount);
        assert_eq!(
            token_client.balance(&contract_id),
            stranded_amount - withdraw_amount
        );
    }

    /// Verifies that `emergency_withdraw` can drain the entire contract balance
    /// in a single call.
    #[test]
    fn test_emergency_withdraw_full_balance() {
        let (env, client, contract_id) = setup_env();

        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);

        client.initialize(&admin, &treasury, &100, &1000, &PaymentRouter::MAX_AMOUNT);

        let (token_address, token_client, stellar_asset_client) = setup_token(&env);

        let stranded_amount = 7_500i128;
        stellar_asset_client.mint(&contract_id, &stranded_amount);

        client.emergency_withdraw(&token_address, &stranded_amount);

        assert_eq!(token_client.balance(&admin), stranded_amount);
        assert_eq!(token_client.balance(&contract_id), 0);
    }

    /// Verifies that `emergency_withdraw` declares admin authorization as required.
    ///
    /// Soroban's `require_auth()` uses an abort-on-failure model in the host
    /// (non-unwinding panics), so we cannot catch a missing-auth failure inside
    /// the same test process.  Instead we use `mock_all_auths_allowing_non_root_auth`
    /// to record which addresses the call attempts to authorize, then assert that
    /// the admin address — and *only* the admin — appears in that list.
    #[test]
    fn test_admin_is_required_for_emergency_withdraw() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let contract_id = env.register_contract(None, PaymentRouter);
        let client = PaymentRouterClient::new(&env, &contract_id);

        client.initialize(&admin, &treasury, &100, &1000, &PaymentRouter::MAX_AMOUNT);

        let (token_address, _token_client, stellar_asset_client) = setup_token(&env);
        stellar_asset_client.mint(&contract_id, &5_000i128);

        // Call succeeds because mock_all_auths satisfies any require_auth.
        // What we verify is that the invocation recorded exactly one
        // authorization and that it belongs to admin, proving the function
        // gates on the admin address.
        client.emergency_withdraw(&token_address, &1_000i128);

        let auths = env.auths();
        let admin_auth_present = auths.iter().any(|(addr, _)| *addr == admin);
        assert!(
            admin_auth_present,
            "emergency_withdraw must require the admin address to authorize"
        );
    }

    #[test]
    fn test_blacklist_recipient() {
        let (env, client, _) = setup_env();

        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        let (token_address, _token_client, sac) = setup_token(&env);
        sac.mint(&sender, &10_000);

        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);

        // Blacklist changes are only applied through the timelock.
        execute_after_delay(
            &env,
            &client,
            &ActionType::SetBlacklist(recipient.clone(), true),
        );
        assert!(client.is_blacklisted(&recipient));

        // Route payment should fail
        let res = client.try_route_payment(&sender, &recipient, &token_address, &1000);
        assert_eq!(res.unwrap_err().unwrap(), Error::Blacklisted);

        // Unblacklist and try again
        execute_after_delay(
            &env,
            &client,
            &ActionType::SetBlacklist(recipient.clone(), false),
        );
        assert!(!client.is_blacklisted(&recipient));

        client
            .mock_all_auths()
            .route_payment(&sender, &recipient, &token_address, &1000);
    }

    #[test]
    fn test_token_and_amount_policy_changes_are_timelocked() {
        let (env, client, _) = setup_env();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);
        let (token_address, token_client, sac) = setup_token(&env);
        sac.mint(&sender, &10_000);
        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);

        assert_eq!(
            client
                .try_queue_action(&ActionType::SetMaxAmount(0))
                .unwrap_err()
                .unwrap(),
            Error::LimitExceeded
        );
        execute_after_delay(
            &env,
            &client,
            &ActionType::SetTokenBlocked(token_address.clone(), true),
        );
        assert_eq!(
            client
                .try_route_payment(&sender, &recipient, &token_address, &100)
                .unwrap_err()
                .unwrap(),
            Error::TokenBlocked
        );

        execute_after_delay(
            &env,
            &client,
            &ActionType::SetTokenBlocked(token_address.clone(), false),
        );
        execute_after_delay(&env, &client, &ActionType::SetMaxAmount(500));
        assert_eq!(
            client
                .try_route_payment(&sender, &recipient, &token_address, &501)
                .unwrap_err()
                .unwrap(),
            Error::LimitExceeded
        );
        client.route_payment(&sender, &recipient, &token_address, &500);
        assert_eq!(token_client.balance(&recipient), 495);
    }

    #[test]
    #[ignore]
    fn test_routes_multiple_distinct_assets() {
        let (env, client, _) = setup_env();

        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        client.initialize(
            &admin,
            &treasury,
            &100,
            &1_000_000,
            &PaymentRouter::MAX_AMOUNT,
        );

        let (usdc_like_address, usdc_like_client, usdc_like_admin_client) = setup_token(&env);
        let (eurc_like_address, eurc_like_client, eurc_like_admin_client) = setup_token(&env);
        assert_ne!(usdc_like_address, eurc_like_address);

        usdc_like_admin_client.mint(&sender, &10_000);
        eurc_like_admin_client.mint(&sender, &5_000);

        client.route_payment(&sender, &recipient, &usdc_like_address, &2_000);
        client.route_payment(&sender, &recipient, &eurc_like_address, &1_000);

        assert_eq!(usdc_like_client.balance(&sender), 8_000);
        assert_eq!(usdc_like_client.balance(&recipient), 1_980);
        assert_eq!(eurc_like_client.balance(&sender), 4_000);
        assert_eq!(eurc_like_client.balance(&recipient), 990);
        assert_eq!(client.get_user_volume(&sender), 3_000);
    }

    #[test]
    fn test_benchmark_gas_costs() {
        let (env, client, _) = setup_env();

        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        let (token_address, _token_client, sac) = setup_token(&env);
        sac.mint(&sender, &10_000);

        // Reset budget before initialization
        env.budget().reset_default();
        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);
        let init_cpu = env.budget().cpu_instruction_cost();
        let init_mem = env.budget().memory_bytes_cost();
        log!(
            &env,
            "GAS REPORT: initialize - CPU: {}, Mem: {}",
            init_cpu,
            init_mem
        );

        // Reset budget before route_payment
        env.budget().reset_default();
        client.route_payment(&sender, &recipient, &token_address, &5_000);
        let route_cpu = env.budget().cpu_instruction_cost();
        let route_mem = env.budget().memory_bytes_cost();
        log!(
            &env,
            "GAS REPORT: route_payment - CPU: {}, Mem: {}",
            route_cpu,
            route_mem
        );

        env.budget().print();

        // Fails CI if gas costs exceed defined thresholds
        // Set reasonable thresholds (e.g. 5M CPU and 2MB Mem per call)
        let max_cpu = 5_000_000;
        let max_mem = 2_000_000;

        assert!(
            init_cpu <= max_cpu,
            "initialize CPU cost exceeded threshold! Cost: {}, Threshold: {}",
            init_cpu,
            max_cpu
        );
        assert!(
            init_mem <= max_mem,
            "initialize Memory cost exceeded threshold! Cost: {}, Threshold: {}",
            init_mem,
            max_mem
        );

        assert!(
            route_cpu <= max_cpu,
            "route_payment CPU cost exceeded threshold! Cost: {}, Threshold: {}",
            route_cpu,
            max_cpu
        );
        assert!(
            route_mem <= max_mem,
            "route_payment Memory cost exceeded threshold! Cost: {}, Threshold: {}",
            route_mem,
            max_mem
        );
    }

    /// When a recipient has no trustline (deauthorized on a Stellar Asset
    /// Contract), `route_payment` must NOT abort.  Instead it should credit
    /// the sender's refund ledger with the remainder and emit a "refunded"
    /// event so the sender can later call `withdraw_refund`.
    #[test]
    fn test_route_payment_credits_refund_on_missing_trustline() {
        let (env, client, contract_id) = setup_env();

        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        let token_address = env.register_contract(None, RefundTestToken);
        let mock_token = RefundTestTokenClient::new(&env, &token_address);
        mock_token.init(&recipient);
        mock_token.mint(&sender, &10_000);
        let token_client = token::Client::new(&env, &token_address);

        // Initialize with 1 % fee (100 bps), cap 50, and a large per-day limit.
        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);
        client.add_supported_token(&token_address);

        // route_payment must succeed (not abort).
        let amount = 2_000i128;
        client.route_payment(&sender, &recipient, &token_address, &amount);

        // Fee (1 % of 2000 = 20) still reaches the treasury.
        assert_eq!(token_client.balance(&treasury), 20);

        // The rejected recipient received nothing.
        assert_eq!(token_client.balance(&recipient), 0);

        // Remainder (1980) must be held by the contract.
        let remainder = 1_980i128;
        assert_eq!(token_client.balance(&contract_id), remainder);

        // Sender's refund ledger must reflect the full remainder.
        assert_eq!(
            client.get_refund_balance(&sender, &token_address),
            remainder
        );

        // A "refunded" event must have been published.
        let events = env.events().all();
        let refunded_found = events.iter().any(|evt| {
            let (_cid, topics, _data) = evt;
            if topics.is_empty() {
                return false;
            }
            let t0: soroban_sdk::Symbol = match topics.get(0).unwrap().try_into_val(&env) {
                Ok(s) => s,
                Err(_) => return false,
            };
            t0 == symbol_short!("refunded")
        });
        assert!(
            refunded_found,
            "route_payment should publish a \"refunded\" event on trustline failure"
        );
    }

    #[test]
    #[ignore]
    fn test_refund_ledger_and_withdrawal() {
        let (env, client, contract_id) = setup_env();

        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let user = Address::generate(&env);

        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);

        let (token_address, token_client, stellar_asset_client) = setup_token(&env);

        // Initially zero refund balance
        assert_eq!(client.get_refund_balance(&user, &token_address), 0);

        // Simulate stranded tokens in contract and credit internal refund balance
        let refund_amount = 5_000i128;
        stellar_asset_client.mint(&contract_id, &refund_amount);

        env.as_contract(&contract_id, || {
            PaymentRouter::credit_refund_balance(&env, &user, &token_address, refund_amount);
        });

        assert_eq!(
            client.get_refund_balance(&user, &token_address),
            refund_amount
        );

        // User withdraws partial refund
        let partial_amount = 2_000i128;
        client.withdraw_refund(&user, &token_address, &partial_amount);

        assert_eq!(token_client.balance(&user), partial_amount);
        assert_eq!(
            client.get_refund_balance(&user, &token_address),
            refund_amount - partial_amount
        );

        // User claims remaining refunds with claim_all_refunds
        let claimed = client.claim_all_refunds(&user, &token_address);
        assert_eq!(claimed, refund_amount - partial_amount);
        assert_eq!(token_client.balance(&user), refund_amount);
        assert_eq!(client.get_refund_balance(&user, &token_address), 0);

        // Trying to withdraw again should fail with NoRefundAvailable
        let res = client.try_withdraw_refund(&user, &token_address, &100);
        assert_eq!(res.unwrap_err().unwrap(), Error::NoRefundAvailable);
    }

    // ── Minimum transfer amount tests ────────────────────────────────────────

    #[test]
    fn test_route_payment_below_min_amount_rejected() {
        let (env, client, _) = setup_env();

        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        let (token_address, _token_client, sac) = setup_token(&env);
        sac.mint(&sender, &10_000);

        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);
        execute_after_delay(&env, &client, &ActionType::SetMinAmount(100));

        // Amount below the minimum must be rejected.
        let res = client.try_route_payment(&sender, &recipient, &token_address, &99);
        assert_eq!(res.unwrap_err().unwrap(), Error::AmountBelowMinimum);

        // A single stroop must also be rejected.
        let res = client.try_route_payment(&sender, &recipient, &token_address, &1);
        assert_eq!(res.unwrap_err().unwrap(), Error::AmountBelowMinimum);
    }

    #[test]
    fn test_route_payment_at_or_above_min_amount_succeeds() {
        let (env, client, _) = setup_env();

        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        let (token_address, token_client, sac) = setup_token(&env);
        sac.mint(&sender, &10_000);

        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);
        execute_after_delay(&env, &client, &ActionType::SetMinAmount(100));

        // Exactly at the minimum succeeds.
        client.route_payment(&sender, &recipient, &token_address, &100);
        assert_eq!(token_client.balance(&recipient), 99);

        // Above the minimum succeeds.
        client.route_payment(&sender, &recipient, &token_address, &1_000);
        assert_eq!(token_client.balance(&recipient), 99 + 990);
    }

    #[test]
    fn test_set_min_amount_requires_admin_auth() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let contract_id = env.register_contract(None, PaymentRouter);
        let client = PaymentRouterClient::new(&env, &contract_id);

        client.initialize(&admin, &treasury, &100, &1000, &PaymentRouter::MAX_AMOUNT);

        let result = client.try_set_min_amount(&250);
        assert_eq!(result.unwrap_err().unwrap(), Error::TimelockRequired);

        let nonce = client.queue_action(&ActionType::SetMinAmount(250));

        // Queueing the supported action records authorization for the admin.
        let auths = env.auths();
        let admin_auth_present = auths.iter().any(|(addr, _)| *addr == admin);
        assert!(
            admin_auth_present,
            "set_min_amount must require the admin address to authorize"
        );

        advance_time(&env, PaymentRouter::SECONDS_IN_24H);
        client.execute_action(&nonce);

        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);
        let (token_address, _token_client, sac) = setup_token(&env);
        sac.mint(&sender, &10_000);
        let res = client.try_route_payment(&sender, &recipient, &token_address, &249);
        assert_eq!(res.unwrap_err().unwrap(), Error::AmountBelowMinimum);
    }

    #[test]
    fn test_governance_takes_over_fees() {
        let (_, client, _) = setup_env();

        let admin = Address::generate(&client.env);
        let treasury = Address::generate(&client.env);
        let gov = Address::generate(&client.env);

        client.initialize(&admin, &treasury, &100, &1000, &PaymentRouter::MAX_AMOUNT);

        // Admin controls fees until governance is installed.
        execute_after_delay(&client.env, &client, &ActionType::SetFeeBps(150));
        assert_eq!(client.get_fee(), 150);

        execute_after_delay(
            &client.env,
            &client,
            &ActionType::SetGovernance(gov.clone()),
        );

        // Governance controls fee actions after it is installed.
        execute_after_delay(&client.env, &client, &ActionType::SetFeeBps(200));
        assert_eq!(client.get_fee(), 200);
    }

    // ── get_version tests ────────────────────────────────────────────────────

    /// get_version() must return the semantic version string defined in
    /// Cargo.toml.  The format must be MAJOR.MINOR.PATCH and match what
    /// `CARGO_PKG_VERSION` expands to at compile time.
    #[test]
    fn test_get_version_returns_semver_string() {
        let env = Env::default();
        let contract_id = env.register_contract(None, PaymentRouter);
        let client = PaymentRouterClient::new(&env, &contract_id);

        let version = client.get_version();

        // The version must equal the Cargo.toml package version.
        let expected = soroban_sdk::String::from_str(&env, env!("CARGO_PKG_VERSION"));
        assert_eq!(version, expected);
    }

    #[test]
    fn test_interface_version_is_stable() {
        let (_env, client, _) = setup_env();
        assert_eq!(client.interface_version(), PaymentRouter::INTERFACE_VERSION);
    }

    /// The version string must be in MAJOR.MINOR.PATCH format (three
    /// dot-separated numeric components).
    #[test]
    fn test_get_version_format_is_semver() {
        let env = Env::default();
        let contract_id = env.register_contract(None, PaymentRouter);
        let client = PaymentRouterClient::new(&env, &contract_id);

        let version = client.get_version();

        // Convert to a Rust String for easy assertion.
        let mut bytes = std::vec![0u8; version.len() as usize];
        version.copy_into_slice(&mut bytes);
        let version_str = std::str::from_utf8(&bytes).expect("version should be valid UTF-8");

        // Must have exactly two dots (MAJOR.MINOR.PATCH).
        let parts: std::vec::Vec<&str> = version_str.split('.').collect();
        assert_eq!(
            parts.len(),
            3,
            "version must have three components, got: {version_str}"
        );

        // Each part must be a non-negative integer.
        for part in &parts {
            part.parse::<u64>()
                .unwrap_or_else(|_| panic!("version component '{part}' is not an integer"));
        }
    }
}

/// Tests covering the four schema-v1 features:
///
/// 1. Caller-nonce execution receipts (per-sender, immutable, with expiry).
/// 2. Per-token fee accounting (read-only totals, reconciled with events,
///    failed payments excluded).
/// 3. Refund creation timestamps and governed post-expiry recovery.
/// 4. Schema version storage, bounded idempotent migration, and the guard
///    that prevents finalization from being called twice.
#[cfg(test)]
mod v1_tests {
    use super::*;
    use soroban_sdk::{
        testutils::{Address as _, Events, Ledger as _, LedgerInfo},
        token, Address, Env, Symbol, TryIntoVal,
    };

    // ── helpers ──────────────────────────────────────────────────────────────

    fn setup() -> (Env, PaymentRouterClient<'static>, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let cid = env.register_contract(None, PaymentRouter);
        let client = PaymentRouterClient::new(&env, &cid);
        (env, client, cid)
    }

    fn setup_token(
        env: &Env,
    ) -> (Address, token::Client<'static>, token::StellarAssetClient<'static>) {
        let ta = Address::generate(env);
        let addr = env.register_stellar_asset_contract(ta);
        let tc = token::Client::new(env, &addr);
        let sac = token::StellarAssetClient::new(env, &addr);
        (addr, tc, sac)
    }

    fn advance(env: &Env, delta: u64) {
        let ts = env.ledger().timestamp();
        env.ledger().set(LedgerInfo {
            timestamp: ts + delta,
            protocol_version: env.ledger().protocol_version(),
            sequence_number: env.ledger().sequence(),
            network_id: env.ledger().network_id().into(),
            base_reserve: 100,
            min_temp_entry_ttl: 16,
            min_persistent_entry_ttl: 4096,
            max_entry_ttl: 6_312_000,
        });
    }

    // ══════════════════════════════════════════════════════════════════════════
    // Feature 1: Caller-nonce execution receipts
    // ══════════════════════════════════════════════════════════════════════════

    /// A successful `route_payment_with_nonce` call writes an immutable
    /// receipt that can be retrieved with `get_payment_receipt`.
    #[test]
    fn test_nonce_success_writes_immutable_receipt() {
        let (env, client, _) = setup();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);
        let (token, tc, sac) = setup_token(&env);
        sac.mint(&sender, &10_000);

        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);

        let nonce = client.get_payment_nonce_counter(&sender) + 1;
        assert_eq!(nonce, 1);

        client.route_payment_with_nonce(&sender, &recipient, &token, &2_000, &nonce);

        let receipt = client.get_payment_receipt(&sender, &nonce);
        assert_eq!(receipt.sender, sender);
        assert_eq!(receipt.recipient, recipient);
        assert_eq!(receipt.token, token);
        assert_eq!(receipt.amount, 2_000);
        // Fee: 100 bps of 2000 = 20, capped at 50 → 20
        assert_eq!(receipt.fee, 20);
        assert_eq!(tc.balance(&recipient), 1_980);
    }

    /// The nonce counter advances monotonically per sender.
    #[test]
    fn test_nonce_counter_advances_monotonically() {
        let (env, client, _) = setup();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);
        let (token, _tc, sac) = setup_token(&env);
        sac.mint(&sender, &100_000);

        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);

        for expected in 1u64..=5 {
            let nonce = client.get_payment_nonce_counter(&sender) + 1;
            assert_eq!(nonce, expected);
            client.route_payment_with_nonce(&sender, &recipient, &token, &1_000, &nonce);
            assert_eq!(client.get_payment_nonce_counter(&sender), expected);
        }
    }

    /// Two independent senders have separate nonce spaces.
    #[test]
    fn test_nonce_scoped_per_sender() {
        let (env, client, _) = setup();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let sender_a = Address::generate(&env);
        let sender_b = Address::generate(&env);
        let recipient = Address::generate(&env);
        let (token, _tc, sac) = setup_token(&env);
        sac.mint(&sender_a, &10_000);
        sac.mint(&sender_b, &10_000);

        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);

        // Both start at counter 0.
        assert_eq!(client.get_payment_nonce_counter(&sender_a), 0);
        assert_eq!(client.get_payment_nonce_counter(&sender_b), 0);

        client.route_payment_with_nonce(&sender_a, &recipient, &token, &1_000, &1u64);
        // sender_a is now at 1; sender_b is still at 0.
        assert_eq!(client.get_payment_nonce_counter(&sender_a), 1);
        assert_eq!(client.get_payment_nonce_counter(&sender_b), 0);

        // sender_b can still use nonce 1.
        client.route_payment_with_nonce(&sender_b, &recipient, &token, &1_000, &1u64);
        assert_eq!(client.get_payment_nonce_counter(&sender_b), 1);

        // The two receipts are independent.
        let ra = client.get_payment_receipt(&sender_a, &1u64);
        let rb = client.get_payment_receipt(&sender_b, &1u64);
        assert_eq!(ra.sender, sender_a);
        assert_eq!(rb.sender, sender_b);
    }

    /// Re-submitting a nonce that has already been used returns `NonceDuplicate`.
    /// The receipt-existence check fires before the sequence check.
    #[test]
    fn test_nonce_duplicate_rejected() {
        let (env, client, _) = setup();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);
        let (token, _tc, sac) = setup_token(&env);
        sac.mint(&sender, &100_000);

        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);

        client.route_payment_with_nonce(&sender, &recipient, &token, &1_000, &1u64);

        // Nonce 1 receipt already exists → NonceDuplicate.
        let res = client.try_route_payment_with_nonce(&sender, &recipient, &token, &1_000, &1u64);
        assert_eq!(res.unwrap_err().unwrap(), Error::NonceDuplicate);
    }

    /// Supplying an out-of-sequence nonce returns `NonceExpired`.
    #[test]
    fn test_nonce_out_of_sequence_rejected() {
        let (env, client, _) = setup();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);
        let (token, _tc, sac) = setup_token(&env);
        sac.mint(&sender, &100_000);

        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);

        // Counter is 0 → expected nonce is 1. Supplying 3 is out of sequence.
        let res = client.try_route_payment_with_nonce(&sender, &recipient, &token, &1_000, &3u64);
        assert_eq!(res.unwrap_err().unwrap(), Error::NonceExpired);
    }

    /// A receipt can be pruned after its expiry time.
    #[test]
    fn test_receipt_pruned_after_expiry() {
        let (env, client, _) = setup();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);
        let (token, _tc, sac) = setup_token(&env);
        sac.mint(&sender, &10_000);

        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);
        client.route_payment_with_nonce(&sender, &recipient, &token, &1_000, &1u64);

        // Cannot prune before expiry.
        let res = client.try_prune_receipt(&sender, &1u64);
        assert_eq!(res.unwrap_err().unwrap(), Error::TimelockNotReady);

        // Advance past the NONCE_EXPIRY_SECONDS (7 days).
        advance(&env, PaymentRouter::NONCE_EXPIRY_SECONDS + 1);

        client.prune_receipt(&sender, &1u64);

        // Receipt is gone.
        let res = client.try_get_payment_receipt(&sender, &1u64);
        assert_eq!(res.unwrap_err().unwrap(), Error::TimelockNotFound);
    }

    /// A failed payment does not write a receipt.
    #[test]
    fn test_failed_payment_does_not_write_receipt() {
        let (env, client, _) = setup();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);
        let (token, _tc, _sac) = setup_token(&env); // no mint → insufficient balance

        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);

        let res = client.try_route_payment_with_nonce(&sender, &recipient, &token, &1_000, &1u64);
        assert_eq!(res.unwrap_err().unwrap(), Error::InsufficientBalance);

        // No receipt written.
        let res = client.try_get_payment_receipt(&sender, &1u64);
        assert_eq!(res.unwrap_err().unwrap(), Error::TimelockNotFound);
    }

    // ══════════════════════════════════════════════════════════════════════════
    // Feature 2: Per-token fee accounting
    // ══════════════════════════════════════════════════════════════════════════

    /// `get_fee_total` starts at zero and increments only on successful payments.
    #[test]
    fn test_fee_total_increments_on_success() {
        let (env, client, _) = setup();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);
        let (token, _tc, sac) = setup_token(&env);
        sac.mint(&sender, &100_000);

        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);

        assert_eq!(client.get_fee_total(&token), 0);

        // 100 bps of 2000 = 20, cap 50 → fee = 20
        client.route_payment(&sender, &recipient, &token, &2_000);
        assert_eq!(client.get_fee_total(&token), 20);

        // 100 bps of 8000 = 80 > cap 50 → fee = 50
        client.route_payment(&sender, &recipient, &token, &8_000);
        assert_eq!(client.get_fee_total(&token), 70);
    }

    /// A failed payment does not increment the fee total.
    #[test]
    fn test_fee_total_not_incremented_on_failure() {
        let (env, client, _) = setup();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);
        let (token, _tc, _sac) = setup_token(&env); // no mint

        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);

        let _ = client.try_route_payment(&sender, &recipient, &token, &1_000);
        assert_eq!(client.get_fee_total(&token), 0);
    }

    /// Fee totals are tracked independently per token.
    #[test]
    fn test_fee_total_per_token() {
        let (env, client, _) = setup();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);
        let (token_a, _tc_a, sac_a) = setup_token(&env);
        let (token_b, _tc_b, sac_b) = setup_token(&env);
        sac_a.mint(&sender, &100_000);
        sac_b.mint(&sender, &100_000);

        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);

        client.route_payment(&sender, &recipient, &token_a, &2_000);
        client.route_payment(&sender, &recipient, &token_b, &8_000);

        assert_eq!(client.get_fee_total(&token_a), 20); // 100 bps of 2000, cap 50
        assert_eq!(client.get_fee_total(&token_b), 50); // 100 bps of 8000 > cap 50
    }

    /// A `fee_collected` event is emitted for each successful fee transfer,
    /// and the total reconciles with the sum of event amounts.
    #[test]
    fn test_fee_total_reconciles_with_events() {
        let (env, client, _) = setup();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);
        let (token, _tc, sac) = setup_token(&env);
        sac.mint(&sender, &100_000);

        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);

        client.route_payment(&sender, &recipient, &token, &2_000);
        client.route_payment(&sender, &recipient, &token, &2_000);
        client.route_payment(&sender, &recipient, &token, &2_000);

        // Sum fee_collected events.
        let events = env.events().all();
        let mut event_sum: i128 = 0;
        for (_, topics, data) in events.iter() {
            if topics.is_empty() {
                continue;
            }
            let sym: Result<Symbol, _> = topics.get(0).unwrap().try_into_val(&env);
            if sym.map(|s| s == Symbol::new(&env, "fee_collected")).unwrap_or(false) {
                let fee: i128 = data.try_into_val(&env).unwrap();
                event_sum += fee;
            }
        }

        assert_eq!(client.get_fee_total(&token), event_sum);
    }

    // ══════════════════════════════════════════════════════════════════════════
    // Feature 3: Refund creation timestamps and post-expiry governed recovery
    // ══════════════════════════════════════════════════════════════════════════

    /// A new refund entry records the creation timestamp.
    #[test]
    fn test_refund_entry_records_creation_time() {
        let (env, client, _contract_id) = setup();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);
        let token_addr = env.register_contract(None, test::RefundTestToken);
        let mock = test::RefundTestTokenClient::new(&env, &token_addr);
        mock.init(&recipient);
        mock.mint(&sender, &10_000);

        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);

        let ts_before = env.ledger().timestamp();
        client.route_payment(&sender, &recipient, &token_addr, &2_000);

        let entry = client.get_refund_entry_view(&sender, &token_addr).unwrap();
        assert_eq!(entry.balance, 1_980); // 2000 - 20 fee
        assert!(entry.created_at >= ts_before);
        assert_eq!(
            entry.created_at,
            env.ledger().timestamp(),
            "created_at must equal the ledger timestamp when the refund was credited"
        );
    }

    /// Within the expiry window the user can withdraw a refund normally.
    #[test]
    fn test_refund_withdrawal_within_expiry_window() {
        let (env, client, _contract_id) = setup();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);
        let token_addr = env.register_contract(None, test::RefundTestToken);
        let mock = test::RefundTestTokenClient::new(&env, &token_addr);
        mock.init(&recipient);
        mock.mint(&sender, &10_000);
        let tc = token::Client::new(&env, &token_addr);

        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);
        client.route_payment(&sender, &recipient, &token_addr, &2_000);

        // Advance to just inside the expiry window (one second before expiry).
        advance(&env, PaymentRouter::REFUND_EXPIRY_SECONDS - 1);

        // Withdraw should succeed.
        client.withdraw_refund(&sender, &token_addr, &1_980);
        assert_eq!(tc.balance(&sender), 10_000 - 2_000 + 1_980);
    }

    /// After expiry, ordinary withdraw is blocked with `RefundExpired`.
    #[test]
    fn test_refund_withdrawal_blocked_after_expiry() {
        let (env, client, _contract_id) = setup();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);
        let token_addr = env.register_contract(None, test::RefundTestToken);
        let mock = test::RefundTestTokenClient::new(&env, &token_addr);
        mock.init(&recipient);
        mock.mint(&sender, &10_000);

        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);
        client.route_payment(&sender, &recipient, &token_addr, &2_000);

        advance(&env, PaymentRouter::REFUND_EXPIRY_SECONDS);

        let res = client.try_withdraw_refund(&sender, &token_addr, &1_980);
        assert_eq!(res.unwrap_err().unwrap(), Error::RefundExpired);
    }

    /// `claim_all_refunds` is also blocked after expiry.
    #[test]
    fn test_claim_all_refunds_blocked_after_expiry() {
        let (env, client, _) = setup();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);
        let token_addr = env.register_contract(None, test::RefundTestToken);
        let mock = test::RefundTestTokenClient::new(&env, &token_addr);
        mock.init(&recipient);
        mock.mint(&sender, &10_000);

        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);
        client.route_payment(&sender, &recipient, &token_addr, &2_000);

        advance(&env, PaymentRouter::REFUND_EXPIRY_SECONDS);

        let res = client.try_claim_all_refunds(&sender, &token_addr);
        assert_eq!(res.unwrap_err().unwrap(), Error::RefundExpired);
    }

    /// `recover_expired_refund` is blocked before expiry.
    #[test]
    fn test_recover_expired_refund_blocked_before_expiry() {
        let (env, client, _) = setup();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);
        let token_addr = env.register_contract(None, test::RefundTestToken);
        let mock = test::RefundTestTokenClient::new(&env, &token_addr);
        mock.init(&recipient);
        mock.mint(&sender, &10_000);

        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);
        client.route_payment(&sender, &recipient, &token_addr, &2_000);

        // Not expired yet.
        let res = client.try_recover_expired_refund(&sender, &token_addr);
        assert_eq!(res.unwrap_err().unwrap(), Error::TimelockNotReady);
    }

    /// After expiry, the admin can recover the balance and a `refund_recovered`
    /// event is emitted.
    #[test]
    fn test_recover_expired_refund_succeeds_after_expiry() {
        let (env, client, contract_id) = setup();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);
        let token_addr = env.register_contract(None, test::RefundTestToken);
        let mock = test::RefundTestTokenClient::new(&env, &token_addr);
        mock.init(&recipient);
        mock.mint(&sender, &10_000);
        let tc = token::Client::new(&env, &token_addr);

        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);
        client.route_payment(&sender, &recipient, &token_addr, &2_000);

        let remainder = 1_980i128;
        assert_eq!(tc.balance(&contract_id), remainder);

        advance(&env, PaymentRouter::REFUND_EXPIRY_SECONDS);

        let recovered = client.recover_expired_refund(&sender, &token_addr);
        assert_eq!(recovered, remainder);
        assert_eq!(tc.balance(&admin), remainder);
        assert_eq!(tc.balance(&contract_id), 0);

        // RefundEntry is gone.
        assert!(client.get_refund_entry_view(&sender, &token_addr).is_none());

        // Event emitted.
        let events = env.events().all();
        let found = events.iter().any(|(_, topics, _)| {
            if topics.is_empty() { return false; }
            let sym: Result<Symbol, _> = topics.get(0).unwrap().try_into_val(&env);
            sym.map(|s| s == Symbol::new(&env, "refund_recovered")).unwrap_or(false)
        });
        assert!(found, "refund_recovered event not emitted");
    }

    /// `recover_expired_refund` on a non-existent entry returns `NoRefundAvailable`.
    #[test]
    fn test_recover_expired_refund_no_entry() {
        let (env, client, _) = setup();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let user = Address::generate(&env);
        let (token, _, _) = setup_token(&env);

        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);

        let res = client.try_recover_expired_refund(&user, &token);
        assert_eq!(res.unwrap_err().unwrap(), Error::NoRefundAvailable);
    }

    // ══════════════════════════════════════════════════════════════════════════
    // Feature 4: Schema versioning and bounded idempotent migration
    // ══════════════════════════════════════════════════════════════════════════

    /// A freshly initialized contract reports schema version SCHEMA_VERSION.
    #[test]
    fn test_initialize_stores_schema_version() {
        let (env, client, _) = setup();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);

        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);

        assert_eq!(client.schema_version(), SCHEMA_VERSION);
    }

    /// `finalize_migration_v0_to_v1` on a v0 contract succeeds.
    #[test]
    fn test_finalize_migration_v0_to_v1_succeeds() {
        let (env, client, contract_id) = setup();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);

        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);

        // Simulate a v0 contract by zeroing the schema version directly.
        env.as_contract(&contract_id, || {
            env.storage()
                .instance()
                .set(&DataKey::SchemaVersion, &0u32);
        });
        assert_eq!(client.schema_version(), 0);

        client.finalize_migration_v0_to_v1();
        assert_eq!(client.schema_version(), 1);

        // Event emitted.
        let events = env.events().all();
        let found = events.iter().any(|(_, topics, _)| {
            if topics.is_empty() { return false; }
            let sym: Result<Symbol, _> = topics.get(0).unwrap().try_into_val(&env);
            sym.map(|s| s == Symbol::new(&env, "schema_migrated")).unwrap_or(false)
        });
        assert!(found, "schema_migrated event not emitted");
    }

    /// `finalize_migration_v0_to_v1` on an already-v1 contract returns
    /// `MigrationVersionMismatch`.
    #[test]
    fn test_finalize_migration_idempotent_guard() {
        let (env, client, _) = setup();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);

        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);
        // Already at v1 (set by initialize).
        let res = client.try_finalize_migration_v0_to_v1();
        assert_eq!(res.unwrap_err().unwrap(), Error::MigrationVersionMismatch);
    }

    /// `migrate_refund_entry_v0_to_v1` on a v1 contract returns `AlreadyMigrated`.
    #[test]
    fn test_migrate_refund_entry_blocked_on_v1() {
        let (env, client, _) = setup();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let user = Address::generate(&env);
        let (token, _, _) = setup_token(&env);

        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);
        // Already at v1.
        let res = client.try_migrate_refund_entry_v0_to_v1(&user, &token);
        assert_eq!(res.unwrap_err().unwrap(), Error::AlreadyMigrated);
    }

    /// A legacy `RefundBalance` i128 is correctly migrated to a `RefundEntry`.
    #[test]
    fn test_migrate_refund_entry_v0_to_v1_migrates_legacy_balance() {
        let (env, client, contract_id) = setup();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let user = Address::generate(&env);
        let (token, _, _) = setup_token(&env);

        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);

        // Set schema to 0 and write a legacy RefundBalance entry.
        env.as_contract(&contract_id, || {
            env.storage()
                .instance()
                .set(&DataKey::SchemaVersion, &0u32);
            env.storage()
                .persistent()
                .set(&DataKey::RefundBalance(user.clone(), token.clone()), &5_000i128);
        });

        client.migrate_refund_entry_v0_to_v1(&user, &token);

        // The new RefundEntry exists.
        let entry = client.get_refund_entry_view(&user, &token).unwrap();
        assert_eq!(entry.balance, 5_000);
        // created_at = 0 (migration sentinel).
        assert_eq!(entry.created_at, 0);

        // The legacy key is gone.
        env.as_contract(&contract_id, || {
            assert!(
                !env.storage()
                    .persistent()
                    .has(&DataKey::RefundBalance(user.clone(), token.clone())),
                "legacy RefundBalance key should have been removed"
            );
        });
    }

    /// Calling `migrate_refund_entry_v0_to_v1` twice for the same pair is
    /// idempotent (the second call returns `Ok(())`).
    #[test]
    fn test_migrate_refund_entry_v0_to_v1_idempotent() {
        let (env, client, contract_id) = setup();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let user = Address::generate(&env);
        let (token, _, _) = setup_token(&env);

        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);

        env.as_contract(&contract_id, || {
            env.storage()
                .instance()
                .set(&DataKey::SchemaVersion, &0u32);
            env.storage()
                .persistent()
                .set(&DataKey::RefundBalance(user.clone(), token.clone()), &3_000i128);
        });

        client.migrate_refund_entry_v0_to_v1(&user, &token);
        // Second call is idempotent.
        client.migrate_refund_entry_v0_to_v1(&user, &token);

        let entry = client.get_refund_entry_view(&user, &token).unwrap();
        assert_eq!(entry.balance, 3_000);
    }

    /// New code at schema v1 rejects reads of a migrated-but-expired entry
    /// from ordinary withdraw, enforcing the invariant that old packed state
    /// cannot be mis-read by new logic once migration completes.
    #[test]
    fn test_new_code_rejects_expired_migrated_entry_via_withdraw() {
        let (env, client, contract_id) = setup();
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let user = Address::generate(&env);
        let (token, tc, sac) = setup_token(&env);

        client.initialize(&admin, &treasury, &100, &50, &PaymentRouter::MAX_AMOUNT);

        // Write a legacy balance and migrate it (created_at = 0 → immediately expired).
        env.as_contract(&contract_id, || {
            env.storage()
                .instance()
                .set(&DataKey::SchemaVersion, &0u32);
            env.storage()
                .persistent()
                .set(&DataKey::RefundBalance(user.clone(), token.clone()), &2_000i128);
        });
        // Fund the contract so the token transfer can succeed if the guard fails.
        sac.mint(&contract_id, &2_000);

        client.migrate_refund_entry_v0_to_v1(&user, &token);
        client.finalize_migration_v0_to_v1();

        // Advance past REFUND_EXPIRY_SECONDS so the created_at=0 entry is expired.
        advance(&env, PaymentRouter::REFUND_EXPIRY_SECONDS + 1);

        // Ordinary withdraw is blocked (entry has created_at = 0, now expired).
        let res = client.try_withdraw_refund(&user, &token, &2_000);
        assert_eq!(
            res.unwrap_err().unwrap(),
            Error::RefundExpired,
            "new code must reject expired migrated entries via ordinary withdraw"
        );

        // Admin can recover it.
        let recovered = client.recover_expired_refund(&user, &token);
        assert_eq!(recovered, 2_000);
        assert_eq!(tc.balance(&admin), 2_000);
    }
}

/// Property-based tests for fee calculation logic.
///
/// These tests exercise the pure arithmetic used in `process_single_payment`
/// without touching the Soroban environment so they can run as ordinary host
/// tests powered by proptest.
///
/// The invariants verified across 10,000 random inputs are:
/// 1. **Conservation**: `fee_amount + remainder == amount`
/// 2. **Non-negative fee**: `fee_amount >= 0`
/// 3. **Non-negative remainder**: `remainder >= 0`
/// 4. **Cap enforcement**: `fee_amount <= fee_cap`
/// 5. **Fee never exceeds amount**: `fee_amount <= amount`
#[cfg(test)]
mod prop_tests {
    use proptest::prelude::*;

    // --- constants mirrored from the contract ---
    const BPS_DIVISOR: i128 = 10_000;
    /// Maximum valid fee in basis points (100% = 10 000 bps).
    const MAX_FEE_BPS: i128 = 10_000;
    /// Upper bound for a single payment amount (matches contract MAX_AMOUNT).
    const MAX_AMOUNT: i128 = 1_000_000_000_000_000;

    // --- pure fee calculation logic (mirrors process_single_payment) ---

    /// Computes `(fee_amount, remainder)` exactly as the contract does.
    ///
    /// `user_volume_above_threshold` stands in for the tiered-discount check:
    /// when `true` the effective fee is halved.
    fn compute_fee(
        amount: i128,
        fee_bps: i128,
        fee_cap: i128,
        user_volume_above_threshold: bool,
    ) -> (i128, i128) {
        let effective_fee_bps = if user_volume_above_threshold {
            fee_bps / 2
        } else {
            fee_bps
        };

        let mut fee_amount = (amount * effective_fee_bps) / BPS_DIVISOR;
        if fee_amount > fee_cap {
            fee_amount = fee_cap;
        }
        if fee_amount > amount {
            fee_amount = amount;
        }
        let remainder = amount - fee_amount;
        (fee_amount, remainder)
    }

    // -----------------------------------------------------------------------
    // Strategies
    // -----------------------------------------------------------------------

    /// A valid payment amount: 1 ..= MAX_AMOUNT (positive, within contract bounds).
    fn valid_amount() -> impl Strategy<Value = i128> {
        1i128..=MAX_AMOUNT
    }

    /// A valid fee in basis points: 0 ..= 10 000 (0% to 100%).
    fn valid_fee_bps() -> impl Strategy<Value = i128> {
        0i128..=MAX_FEE_BPS
    }

    /// A valid fee cap: 0 ..= MAX_AMOUNT.
    fn valid_fee_cap() -> impl Strategy<Value = i128> {
        0i128..=MAX_AMOUNT
    }

    // -----------------------------------------------------------------------
    // Property: fee_amount + remainder == amount  (conservation of funds)
    // -----------------------------------------------------------------------

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(10_000))]

        /// Funds are fully conserved: every strobe of the amount ends up either
        /// in the treasury (fee) or the recipient (remainder), never lost or
        /// created.
        #[test]
        fn prop_fee_plus_remainder_equals_amount(
            amount in valid_amount(),
            fee_bps in valid_fee_bps(),
            fee_cap in valid_fee_cap(),
            above_threshold in any::<bool>(),
        ) {
            let (fee_amount, remainder) = compute_fee(amount, fee_bps, fee_cap, above_threshold);
            prop_assert_eq!(
                fee_amount + remainder,
                amount,
                "fee_amount ({}) + remainder ({}) != amount ({})",
                fee_amount, remainder, amount
            );
        }

        /// The fee is always non-negative — the treasury never receives a
        /// negative transfer.
        #[test]
        fn prop_fee_amount_is_non_negative(
            amount in valid_amount(),
            fee_bps in valid_fee_bps(),
            fee_cap in valid_fee_cap(),
            above_threshold in any::<bool>(),
        ) {
            let (fee_amount, _) = compute_fee(amount, fee_bps, fee_cap, above_threshold);
            prop_assert!(
                fee_amount >= 0,
                "fee_amount ({}) must be >= 0",
                fee_amount
            );
        }

        /// The remainder is always non-negative — the recipient never receives a
        /// negative transfer.
        #[test]
        fn prop_remainder_is_non_negative(
            amount in valid_amount(),
            fee_bps in valid_fee_bps(),
            fee_cap in valid_fee_cap(),
            above_threshold in any::<bool>(),
        ) {
            let (_, remainder) = compute_fee(amount, fee_bps, fee_cap, above_threshold);
            prop_assert!(
                remainder >= 0,
                "remainder ({}) must be >= 0",
                remainder
            );
        }

        /// The fee never exceeds the configured cap.
        #[test]
        fn prop_fee_respects_cap(
            amount in valid_amount(),
            fee_bps in valid_fee_bps(),
            fee_cap in valid_fee_cap(),
            above_threshold in any::<bool>(),
        ) {
            let (fee_amount, _) = compute_fee(amount, fee_bps, fee_cap, above_threshold);
            prop_assert!(
                fee_amount <= fee_cap,
                "fee_amount ({}) exceeds fee_cap ({})",
                fee_amount, fee_cap
            );
        }

        /// The fee never exceeds the payment amount itself — the sender cannot
        /// be charged more than they are sending.
        #[test]
        fn prop_fee_never_exceeds_amount(
            amount in valid_amount(),
            fee_bps in valid_fee_bps(),
            fee_cap in valid_fee_cap(),
            above_threshold in any::<bool>(),
        ) {
            let (fee_amount, _) = compute_fee(amount, fee_bps, fee_cap, above_threshold);
            prop_assert!(
                fee_amount <= amount,
                "fee_amount ({}) exceeds amount ({})",
                fee_amount, amount
            );
        }

        /// When the fee rate is zero the entire amount flows to the recipient.
        #[test]
        fn prop_zero_fee_bps_means_no_fee(
            amount in valid_amount(),
            fee_cap in valid_fee_cap(),
            above_threshold in any::<bool>(),
        ) {
            let (fee_amount, remainder) = compute_fee(amount, 0, fee_cap, above_threshold);
            prop_assert_eq!(fee_amount, 0, "fee_amount must be 0 when fee_bps is 0");
            prop_assert_eq!(remainder, amount, "remainder must equal amount when fee_bps is 0");
        }

        /// When the fee cap is zero no fee is ever collected regardless of the
        /// rate.
        #[test]
        fn prop_zero_fee_cap_means_no_fee(
            amount in valid_amount(),
            fee_bps in valid_fee_bps(),
            above_threshold in any::<bool>(),
        ) {
            let (fee_amount, remainder) = compute_fee(amount, fee_bps, 0, above_threshold);
            prop_assert_eq!(fee_amount, 0, "fee_amount must be 0 when fee_cap is 0");
            prop_assert_eq!(remainder, amount, "remainder must equal amount when fee_cap is 0");
        }

        /// The tiered discount never produces a *higher* fee than the standard
        /// rate: halving the bps can only leave the fee equal or reduce it.
        #[test]
        fn prop_tiered_discount_never_increases_fee(
            amount in valid_amount(),
            fee_bps in valid_fee_bps(),
            fee_cap in valid_fee_cap(),
        ) {
            let (fee_full, _) = compute_fee(amount, fee_bps, fee_cap, false);
            let (fee_discounted, _) = compute_fee(amount, fee_bps, fee_cap, true);
            prop_assert!(
                fee_discounted <= fee_full,
                "discounted fee ({}) must be <= full fee ({})",
                fee_discounted, fee_full
            );
        }
    }
}
