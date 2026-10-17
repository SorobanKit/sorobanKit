/**
 * Shared pagination helper.
 *
 * Computes pagination metadata from a page/pageSize pair and a total count.
 * Prisma `count()` queries can occasionally return unexpected values (e.g. a
 * negative number due to a race condition), so the raw total is clamped to a
 * non-negative value before deriving `totalPages`.
 */

function paginate({ page = 1, pageSize = 10, total = 0 } = {}) {
  const safePage = Number.isFinite(page) && page > 0 ? Math.floor(page) : 1;
  const safePageSize =
    Number.isFinite(pageSize) && pageSize > 0 ? Math.floor(pageSize) : 10;

  const rawTotal = Number.isFinite(total) ? total : 0;

  if (rawTotal < 0) {
    console.warn(
      `[pagination] Received negative total (${rawTotal}); clamping to 0.`
    );
  }

  const safeTotal = Math.max(0, rawTotal);
  const totalPages = Math.ceil(safeTotal / safePageSize);

  return {
    page: safePage,
    pageSize: safePageSize,
    total: safeTotal,
    totalPages,
  };
}

/**
 * Clamps a numeric value between minimum and maximum bounds.
 * Coerces string values to numbers. Returns min for non-numeric or non-finite inputs.
 *
 * @param {number|string} value - The input value to clamp.
 * @param {number} [min=1] - The lower bound (inclusive).
 * @param {number} [max=100] - The upper bound (inclusive).
 * @returns {number} The clamped value within [min, max].
 */
function clamp(value, min = 1, max = 100) {
  if (value === null || value === undefined || (typeof value === 'string' && value.trim() === '')) {
    return min;
  }
  const num = Number(value);
  if (!Number.isFinite(num)) {
    return min;
  }
  if (num < min) {
    return min;
  }
  if (num > max) {
    return max;
  }
  return num;
}

module.exports = { paginate, clamp };
