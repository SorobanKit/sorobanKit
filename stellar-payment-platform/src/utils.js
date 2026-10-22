'use strict';

const DEFAULT_FEDERATION_DOMAIN = 'localhost';

const VALID_MEMO_TYPES = ['text', 'id', 'hash'];
const MEMO_ID_RE = /^\d+$/;
const MEMO_HASH_RE = /^[0-9a-fA-F]{64}$/;

const RESERVED_NAMES = ['admin', 'root', 'support', 'system', 'stellar', 'api', 'help'];

// #613 — an address may carry at most this many federation usernames
// (one primary plus up to four aliases).
const MAX_USERNAMES_PER_ADDRESS = 5;

// #613 — Prisma orderBy that resolves an address to its primary username:
// the row flagged primary wins, with oldest-registered as the tiebreak so
// the result stays stable if no row is flagged (e.g. legacy data).
const PRIMARY_USERNAME_ORDER = [{ isPrimary: 'desc' }, { createdAt: 'asc' }];

function normalizeNameTag(value) {
  const trimmed = typeof value === 'string' ? value.trim() : '';
  if (!trimmed) return '';
  return trimmed.includes('*') ? trimmed : `${trimmed}*${DEFAULT_FEDERATION_DOMAIN}`;
}

function validateMemo(memoType, memo) {
  const isMemoEmpty = memo === undefined || memo === null || memo === '';
  const isTypeEmpty = memoType === undefined || memoType === null || memoType === '';

  if (isTypeEmpty && isMemoEmpty) return null;
  if (!isTypeEmpty && isMemoEmpty) return 'memo is required when memo_type is provided.';
  if (isTypeEmpty && !isMemoEmpty) return 'memo_type is required when memo is provided.';
  if (!VALID_MEMO_TYPES.includes(memoType)) {
    return `memo_type must be one of: ${VALID_MEMO_TYPES.join(', ')}.`;
  }
  if (typeof memo !== 'string') {
    return 'memo must be a string.';
  }
  if (memoType === 'text') {
    // Check if memo contains the replacement character (U+FFFD), which indicates
    // that invalid UTF-8 bytes were decoded by Node.js. We reject these because
    // the original client data was not valid UTF-8 and would cause issues with
    // the Stellar SDK downstream.
    if (memo.includes('\uFFFD')) {
      return 'memo of type text must contain valid UTF-8 characters.';
    }
    // Validate that when encoded as UTF-8, it doesn't exceed 28 bytes
    const byteLength = Buffer.byteLength(memo, 'utf8');
    if (byteLength > 28) {
      return 'memo of type text must not exceed 28 bytes.';
    }
  }
  if (memoType === 'id') {
    if (!MEMO_ID_RE.test(memo) || BigInt(memo) > 18446744073709551615n) {
      return 'memo of type id must be a valid 64-bit unsigned integer.';
    }
  }
  if (memoType === 'hash' && !MEMO_HASH_RE.test(memo)) {
    return 'memo of type hash must be a 64-character hex string (32 bytes).';
  }
  return null;
}

const USER_DATABASE = {
  'client*localhost': 'GAPUQZH3WZUXHEMUGZN5ZYU4D4GHCFEMOGUINU6MF345GBD2QXNYYIEQ',
  'lekan*localhost': 'GAPUQZH3WZUXHEMUGZN5ZYU4D4GHCFEMOGUINU6MF345GBD2QXNYYIEQ',
};

// #175 — exponential backoff with jitter for webhook retry scheduling.
// Delay for `attempt` (1-based) is base * factor^(attempt-1), capped at `max`,
// then randomised by ±`jitter` (fraction, default 0.1 = ±10%).
const DEFAULT_RETRY_FACTOR = 2;
const DEFAULT_RETRY_JITTER = 0.1;

function exponentialDelay(attempt, base, max, jitter = DEFAULT_RETRY_JITTER, factor = DEFAULT_RETRY_FACTOR) {
  const safeAttempt = Number.isFinite(attempt) && attempt > 0 ? Math.floor(attempt) : 1;
  const safeBase = Number.isFinite(base) && base > 0 ? base : 0;
  const safeMax = Number.isFinite(max) && max > 0 ? max : Infinity;
  const safeFactor = Number.isFinite(factor) && factor > 0 ? factor : DEFAULT_RETRY_FACTOR;
  const safeJitter = Number.isFinite(jitter) && jitter >= 0 ? jitter : DEFAULT_RETRY_JITTER;

  const raw = safeBase * Math.pow(safeFactor, safeAttempt - 1);
  const capped = Math.min(raw, safeMax);
  const spread = capped * safeJitter;
  const jittered = capped + (Math.random() * 2 - 1) * spread;
  return Math.max(0, Math.round(jittered));
}

const shouldFallbackToLocalRegistry = (error) => {
  const code = typeof error?.code === 'string' ? error.code : '';
  const message = typeof error?.message === 'string' ? error.message : '';
  return (
    code.startsWith('P10') ||
    ['P2021', 'P2023', 'P2028', 'P2001'].includes(code) ||
    /DATABASE_URL|connect|relation|table|timeout/i.test(message)
  );
};

module.exports = {
  normalizeNameTag,
  validateMemo,
  RESERVED_NAMES,
  MAX_USERNAMES_PER_ADDRESS,
  PRIMARY_USERNAME_ORDER,
  VALID_MEMO_TYPES,
  USER_DATABASE,
  shouldFallbackToLocalRegistry,
  exponentialDelay,
  DEFAULT_RETRY_FACTOR,
  DEFAULT_RETRY_JITTER,
};
