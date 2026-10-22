'use strict';

const { ApiError } = require('../errors');

/**
 * Valid state transitions for a payment intent.
 *
 * Terminal states (COMPLETED, FAILED, CANCELLED) have no outgoing transitions.
 */
const VALID_TRANSITIONS = Object.freeze({
  PENDING: Object.freeze(['PROCESSING', 'CANCELLED']),
  PROCESSING: Object.freeze(['COMPLETED', 'FAILED']),
  COMPLETED: Object.freeze([]),
  FAILED: Object.freeze([]),
  CANCELLED: Object.freeze([]),
});

const ALL_STATUSES = Object.freeze(Object.keys(VALID_TRANSITIONS));

/**
 * Returns true if transitioning from `from` to `to` is allowed.
 *
 * @param {string} from
 * @param {string} to
 * @returns {boolean}
 */
function isValidTransition(from, to) {
  const allowed = VALID_TRANSITIONS[from];
  if (!allowed) {
    return false;
  }
  return allowed.includes(to);
}

/**
 * Throws an ApiError('CONFLICT') if the transition is not allowed.
 *
 * @param {string} from
 * @param {string} to
 * @param {string} [paymentIntentId]
 */
function assertValidTransition(from, to, paymentIntentId) {
  if (!isValidTransition(from, to)) {
    const idSuffix = paymentIntentId ? ` for payment intent ${paymentIntentId}` : '';
    throw new ApiError(
      'CONFLICT',
      `Invalid payment intent status transition from ${from} to ${to}${idSuffix}`
    );
  }
}

/**
 * Builds a Prisma `where` clause that only matches the record if it is still
 * in the expected `from` status. This makes the update atomic: if another
 * writer changed the status in the meantime, the update matches zero rows.
 *
 * @param {string} id
 * @param {string} from
 * @returns {object}
 */
function buildGuardedWhere(id, from) {
  return { id, status: from };
}

/**
 * Performs a guarded status update on a payment intent.
 *
 * The update is concurrency-safe: the `where` clause includes the expected
 * current status, so two concurrent updates from the same source status cannot
 * both succeed. The loser observes `count === 0` and receives a CONFLICT.
 *
 * @param {object} prisma - Prisma client (or transaction client)
 * @param {object} params
 * @param {string} params.id - Payment intent id
 * @param {string} params.to - Target status
 * @param {object} [params.data] - Additional fields to update
 * @returns {Promise<object>} The updated payment intent
 */
async function updatePaymentIntentStatus(prisma, { id, to, data = {} }) {
  if (!ALL_STATUSES.includes(to)) {
    throw new ApiError('CONFLICT', `Unknown payment intent status: ${to}`);
  }

  const current = await prisma.paymentIntent.findUnique({
    where: { id },
    select: { id: true, status: true },
  });

  if (!current) {
    throw new ApiError('NOT_FOUND', `Payment intent ${id} not found`);
  }

  assertValidTransition(current.status, to, id);

  const result = await prisma.paymentIntent.updateMany({
    where: buildGuardedWhere(id, current.status),
    data: { ...data, status: to },
  });

  if (!result || result.count === 0) {
    // Another writer changed the status between our read and write.
    throw new ApiError(
      'CONFLICT',
      `Payment intent ${id} was modified concurrently; status transition to ${to} rejected`
    );
  }

  return prisma.paymentIntent.findUnique({ where: { id } });
}

/**
 * Concurrency-safe transition that performs the read, validation, and write
 * inside a single transaction using a conditional update. Prefer this when a
 * transaction client is available.
 *
 * @param {object} prisma - Prisma client
 * @param {object} params
 * @param {string} params.id
 * @param {string} params.to
 * @param {object} [params.data]
 * @returns {Promise<object>}
 */
async function updatePaymentIntentStatusTx(prisma, { id, to, data = {} }) {
  if (!ALL_STATUSES.includes(to)) {
    throw new ApiError('CONFLICT', `Unknown payment intent status: ${to}`);
  }

  return prisma.$transaction(async (tx) => {
    const current = await tx.paymentIntent.findUnique({
      where: { id },
      select: { id: true, status: true },
    });

    if (!current) {
      throw new ApiError('NOT_FOUND', `Payment intent ${id} not found`);
    }

    assertValidTransition(current.status, to, id);

    const result = await tx.paymentIntent.updateMany({
      where: buildGuardedWhere(id, current.status),
      data: { ...data, status: to },
    });

    if (!result || result.count === 0) {
      throw new ApiError(
        'CONFLICT',
        `Payment intent ${id} was modified concurrently; status transition to ${to} rejected`
      );
    }

    return tx.paymentIntent.findUnique({ where: { id } });
  });
}

module.exports = {
  VALID_TRANSITIONS,
  ALL_STATUSES,
  isValidTransition,
  assertValidTransition,
  buildGuardedWhere,
  updatePaymentIntentStatus,
  updatePaymentIntentStatusTx,
};