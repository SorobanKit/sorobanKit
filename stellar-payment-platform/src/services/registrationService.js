const { StrKey } = require('@stellar/stellar-sdk');
const { prisma, withTransaction } = require('../../prismaClient');
const { verifyMultiSignerThreshold } = require('../multisigner-verifier');
const { logger } = require('../logger');
const { recordActivity, ACTIVITY_ACTIONS } = require('./activityService');

async function transferAccount(username, oldAddress, newAddress, oldSignature, newSignature) {
  if (!username) {
    const error = new Error('Username is required');
    error.statusCode = 400;
    throw error;
  }

  // Normalise the username to lowercase so lookups are consistent regardless
  // of the casing the caller supplied (federation addresses are case-insensitive).
  const normalizedUsername = username.toLowerCase().trim();

  if (!oldAddress || !newAddress) {
    const error = new Error('Both oldAddress and newAddress are required');
    error.statusCode = 400;
    throw error;
  }

  if (!StrKey.isValidEd25519PublicKey(newAddress)) {
    const error = new Error('Invalid Stellar Public Key format for new address');
    error.statusCode = 400;
    throw error;
  }

  if (!oldSignature || !newSignature) {
    const error = new Error('Signatures from both old and new addresses are required');
    error.statusCode = 400;
    throw error;
  }

  // 1. Verify old address exists and matches username
  const user = await prisma.user.findUnique({
    where: { username: normalizedUsername }
  });

  if (!user) {
    const error = new Error('User not found');
    error.statusCode = 404;
    throw error;
  }

  if (user.address !== oldAddress) {
    const error = new Error('Old address does not match current record');
    error.statusCode = 400;
    throw error;
  }

  // 2. Verify old signature against the current address
  const oldVerification = await verifyMultiSignerThreshold(oldAddress, [oldSignature], {
    operationType: 'management',
  });
  if (!oldVerification.success) {
    const error = new Error(oldVerification.errorMessage || 'Signature verification failed for the current address');
    error.statusCode = 401;
    throw error;
  }

  // 3. Verify new signature against the new address
  const newVerification = await verifyMultiSignerThreshold(newAddress, [newSignature], {
    operationType: 'management',
  });
  if (!newVerification.success) {
    const error = new Error(newVerification.errorMessage || 'Signature verification failed for the new address');
    error.statusCode = 401;
    throw error;
  }

  // 4. Atomically update the address in Prisma
  const updatedUser = await prisma.user.update({
    where: { username: normalizedUsername },
    data: { address: newAddress },
  });

  logger.info(`Transferred username ${normalizedUsername} from ${oldAddress} to ${newAddress}`);

  // The ETag-based federation cache will automatically be invalidated for both addresses
  // because the underlying DB data has changed. No manual cache purging is needed.

  return updatedUser;
}

/**
 * Soft-deletes a username (alias or primary) and emits an activity log entry.
 *
 * When the removed username is a non-primary alias the action is
 * ALIAS_REMOVED so the activity trail distinguishes alias removals from
 * full account deletions (USER_UNREGISTERED).  The address and the alias
 * username are both written to metadata so reverse-federation lookups can be
 * audited.
 *
 * @param {string} username  - Normalised username to remove.
 * @param {object} [req]     - Express request object (for IP / user-agent).
 * @returns {Promise<{username: string, address: string, isPrimary: boolean}>}
 */
async function removeAlias(username, req) {
  const existing = await prisma.user.findFirst({
    where: { username, deletedAt: null },
  });

  if (!existing) {
    const err = new Error('Username not found or already deleted');
    err.statusCode = 404;
    throw err;
  }

  await withTransaction(async (tx) => {
    await tx.user.update({
      where: { username },
      data: { deletedAt: new Date() },
    });
  });

  const action = existing.isPrimary
    ? ACTIVITY_ACTIONS.USER_UNREGISTERED
    : ACTIVITY_ACTIONS.ALIAS_REMOVED;

  await recordActivity(prisma, {
    username,
    action,
    metadata: {
      username,
      address: existing.address,
      is_primary: existing.isPrimary,
    },
    req,
  });

  logger.info(`Removed ${existing.isPrimary ? 'primary account' : 'alias'} ${username} (address: ${existing.address})`);

  return { username, address: existing.address, isPrimary: existing.isPrimary };
}

module.exports = {
  transferAccount,
  removeAlias,
};
