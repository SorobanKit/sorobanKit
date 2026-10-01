/**
 * Multi-Signer Verification Module
 * Handles threshold signature validation for enterprise accounts with multiple signers
 * 
 * Fetches account details from Horizon network and validates that provided signatures
 * meet the minimum threshold requirements based on ledger configuration.
 */

const { loadAccount, HORIZON_BASE } = require('./services/stellarService');

// Timeout for individual Horizon account-fetch calls (ms).
// Falls back to the circuit-breaker timeout when not set, but an explicit cap
// here ensures a hanging Horizon request is cancelled well before it can
// exhaust the Node.js event loop under concurrent load.
const HORIZON_FETCH_TIMEOUT_MS =
  parseInt(process.env.HORIZON_FETCH_TIMEOUT_MS, 10) || 5000;

/**
 * Races a promise against a timeout, rejecting with a descriptive error when
 * the deadline is reached before the promise settles.
 *
 * @param {Promise<*>} promise - The promise to race.
 * @param {number} ms - Timeout in milliseconds.
 * @param {string} label - Short description used in the rejection message.
 * @returns {Promise<*>}
 */
function withTimeout(promise, ms, label) {
  let timer;
  const timeout = new Promise((_, reject) => {
    timer = setTimeout(
      () => reject(new Error(`${label} timed out after ${ms}ms`)),
      ms,
    );
  });
  return Promise.race([promise, timeout]).finally(() => clearTimeout(timer));
}

/**
 * Fetches account details from Horizon network including signer configuration.
 * Uses the circuit-breaker–protected `loadAccount` helper so a Horizon outage
 * fast-fails instead of hanging until the TCP timeout.
 *
 * An explicit per-call timeout (HORIZON_FETCH_TIMEOUT_MS, default 5 000 ms)
 * is applied on top of the circuit-breaker timeout so a single slow Horizon
 * response cannot block the event loop indefinitely.
 *
 * @param {string} accountId - The Stellar account public key
 * @param {string} horizonUrl - Optional custom Horizon URL (unused; kept for API compat)
 * @returns {Promise<Object>} Account object with signers array and thresholds
 * @throws {Error} If account not found, network error, or fetch times out
 */
async function fetchAccountSigners(accountId, _horizonUrl) {
  try {
    const account = await withTimeout(
      loadAccount(accountId),
      HORIZON_FETCH_TIMEOUT_MS,
      `Horizon loadAccount(${accountId})`,
    );

    return {
      accountId: account.id,
      signers: account.signers,
      thresholds: {
        low_threshold: account.thresholds.low_threshold,
        med_threshold: account.thresholds.med_threshold,
        high_threshold: account.thresholds.high_threshold,
      },
      sequence: account.sequence,
      balances: account.balances,
    };
  } catch (error) {
    if (error.response?.status === 404) {
      throw new Error(`Account not found on Horizon: ${accountId}`, { cause: error });
    }
    throw new Error(`Failed to fetch account signers: ${error.message}`, { cause: error });
  }
}

/**
 * Calculates total signature weight from provided public keys
 * @param {Array<string>} signaturePublicKeys - Array of signer public keys
 * @param {Array<Object>} accountSigners - Account signers from Horizon (signers array)
 * @returns {Object} { totalWeight: number, signatureDetails: Array }
 */
function calculateSignatureWeight(signaturePublicKeys, accountSigners) {
  const signatureDetails = [];
  let totalWeight = 0;

  // Build a map of public key -> weight from account signers
  const signerMap = new Map();
  for (const signer of accountSigners) {
    signerMap.set(signer.key, signer.weight);
  }

  // Calculate weight for each provided signature
  for (const pubKey of signaturePublicKeys) {
    const weight = signerMap.get(pubKey) || 0;
    signatureDetails.push({
      publicKey: pubKey,
      weight: weight,
      isValid: weight > 0,
    });
    if (weight > 0) {
      totalWeight += weight;
    }
  }

  return {
    totalWeight,
    signatureDetails,
  };
}

/**
 * Determines which threshold should be applied based on operation type
 * Uses: low_threshold (payment), med_threshold (management), high_threshold (highest security)
 * @param {string} operationType - Type of operation: 'payment', 'management', or 'high'
 * @param {Object} thresholds - Thresholds object { low_threshold, med_threshold, high_threshold }
 * @returns {number} The appropriate threshold value
 */
function getApplicableThreshold(operationType = 'payment', thresholds) {
  switch (operationType) {
    case 'management':
      return thresholds.med_threshold;
    case 'high':
      return thresholds.high_threshold;
    case 'payment':
    default:
      return thresholds.low_threshold;
  }
}

/**
 * Returns the sequence number the next transaction must carry (current + 1).
 * Sequence numbers are 64-bit, so BigInt is used to avoid precision loss.
 * @param {string|number|bigint} accountSequence - Current account sequence from Horizon
 * @returns {string}
 */
function nextSequence(accountSequence) {
  return (BigInt(accountSequence) + 1n).toString();
}

/**
 * @param {string|number|bigint} transactionSequence
 * @param {string|number|bigint} accountSequence
 * @returns {boolean} True only when transactionSequence === accountSequence + 1
 */
function isExpectedSequence(transactionSequence, accountSequence) {
  try {
    return BigInt(transactionSequence) === BigInt(accountSequence) + 1n;
  } catch {
    return false;
  }
}

/**
 * Verifies that provided signatures meet the account's signing requirements
 * @param {string} accountId - The Stellar account public key
 * @param {Array<string>} signaturePublicKeys - Array of public keys that signed
 * @param {Object} options - Verification options
 * @param {string} options.operationType - Type of operation ('payment', 'management', 'high')
 * @param {string} options.horizonUrl - Custom Horizon URL
 * @param {string|number|bigint} [options.transactionSequence] - Sequence number of the signed
 *   transaction. When provided it must equal the account's current Horizon sequence + 1,
 *   otherwise verification fails (guards against replay of stale signed transactions).
 * @returns {Promise<Object>} Verification result with details
 * @throws {Error} If verification fails
 */
async function verifyMultiSignerThreshold(accountId, signaturePublicKeys = [], options = {}) {
  const {
    operationType = 'payment',
    horizonUrl = HORIZON_BASE,
    transactionSequence,
  } = options;

  // Validate inputs
  if (!accountId || typeof accountId !== 'string') {
    throw new Error('Invalid account ID provided');
  }

  if (!Array.isArray(signaturePublicKeys) || signaturePublicKeys.length === 0) {
    throw new Error('At least one signature is required for verification');
  }

  // Remove duplicates
  const uniqueSignatures = [...new Set(signaturePublicKeys)];

  // Fetch account details from Horizon
  const accountDetails = await fetchAccountSigners(accountId, horizonUrl);
  
  // Determine applicable threshold based on operation type
  const requiredThreshold = getApplicableThreshold(operationType, accountDetails.thresholds);

  // Calculate total weight from provided signatures
  const { totalWeight, signatureDetails } = calculateSignatureWeight(
    uniqueSignatures,
    accountDetails.signers
  );

  // Check if weight meets threshold
  const meetsThreshold = totalWeight >= requiredThreshold;

  // Reject stale / replayed transactions: the signed transaction's sequence
  // must be exactly the account's current sequence + 1.
  const hasSequence = transactionSequence !== undefined && transactionSequence !== null;
  const sequenceValid = !hasSequence || isExpectedSequence(transactionSequence, accountDetails.sequence);

  return {
    success: meetsThreshold && sequenceValid,
    accountId,
    operationType,
    requiredThreshold,
    totalWeight,
    signatureCount: uniqueSignatures.length,
    uniqueSignerCount: signatureDetails.filter(s => s.isValid).length,
    signatures: signatureDetails,
    thresholds: accountDetails.thresholds,
    signerCount: accountDetails.signers.length,
    errorMessage: !sequenceValid
      ? `invalid sequence number. Expected: ${nextSequence(accountDetails.sequence)}, Provided: ${transactionSequence}`
      : meetsThreshold
        ? null
        : `Insufficient signing weight. Required: ${requiredThreshold}, Provided: ${totalWeight}`,
  };
}

/**
 * Checks if account is single-signer (only master key with weight 1)
 * @param {Array<Object>} signers - Account signers array from Horizon
 * @returns {boolean} True if account has only master signer with weight 1
 */
function isSingleSignerAccount(signers) {
  if (!Array.isArray(signers) || signers.length !== 1) {
    return false;
  }
  
  const masterSigner = signers[0];
  return masterSigner.signer_type === 'ed25519_public_key' && masterSigner.weight === 1;
}

/**
 * Validates that signature matches account's master key
 * For single-signer accounts, this provides backward compatibility
 * @param {string} accountId - The account public key
 * @param {string} signaturePublicKey - The signature to verify
 * @param {string} horizonUrl - Custom Horizon URL
 * @returns {Promise<boolean>} True if signature matches account's master key
 */
async function verifyMasterSignature(accountId, signaturePublicKey, horizonUrl = HORIZON_BASE) {
  try {
    if (accountId === signaturePublicKey) {
      return true;
    }

    // Check if account has this signer registered
    const accountDetails = await fetchAccountSigners(accountId, horizonUrl);
    const signerExists = accountDetails.signers.some(s => s.key === signaturePublicKey);
    
    return signerExists;
  } catch {
    return false;
  }
}

module.exports = {
  fetchAccountSigners,
  calculateSignatureWeight,
  getApplicableThreshold,
  verifyMultiSignerThreshold,
  isSingleSignerAccount,
  verifyMasterSignature,
  // Exported for testing
  withTimeout,
  HORIZON_FETCH_TIMEOUT_MS,
};
