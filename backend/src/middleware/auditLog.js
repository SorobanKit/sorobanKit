const { PrismaClient } = require('@prisma/client');

const prisma = new PrismaClient();

/**
 * Data-subject inventory (issue #395)
 * -----------------------------------
 * User-linked records and how the export/deletion workflow treats them:
 *
 *   - AuditLog (this middleware): EXPORTED in full for the subject's own
 *     entries. On deletion the row is PSEUDONYMIZED, not removed: `userId`
 *     is nulled and `ip` is replaced with a stable hash so the transaction
 *     evidence (action, method, path, tenant, timestamps) survives while the
 *     subject is no longer directly identifiable.
 *   - Registration / activity / federation records: EXPORTED and DELETED by
 *     the async workflow (see the data-subject service).
 *
 * The pseudonymization below is the audit-side half of that contract and is
 * idempotent: re-running it on an already-pseudonymized row is a no-op, so
 * retries and partial failures are safe to resume.
 */

const crypto = require('crypto');

/**
 * Stable, non-reversible pseudonym for a value. Deterministic so repeated
 * runs (retries) produce the same pseudonym and stay idempotent.
 */
function pseudonymize(value) {
  if (value === null || value === undefined || value === '') return null;
  return crypto.createHash('sha256').update(String(value)).digest('hex');
}

/**
 * Pseudonymize every audit entry belonging to a data subject.
 *
 * Preserves required transaction evidence (action, method, path, tenant,
 * timestamps) while removing direct identifiers (userId, ip). Safe to call
 * repeatedly: rows already pseudonymized (userId === null) are skipped.
 *
 * @param {string} userId the data subject whose audit rows to pseudonymize
 * @returns {Promise<number>} number of rows pseudonymized
 */
async function pseudonymizeAuditLogForUser(userId) {
  if (!userId) return 0;

  const rows = await prisma.auditLog.findMany({
    where: { userId },
    select: { id: true, ip: true },
  });

  let updated = 0;
  for (const row of rows) {
    await prisma.auditLog.update({
      where: { id: row.id },
      data: {
        userId: null,
        ip: pseudonymize(row.ip),
      },
    });
    updated += 1;
  }

  return updated;
}

/**
 * Middleware that records admin actions in the AuditLog table.
 *
 * The insert is fire-and-forget: it is scheduled with setImmediate so it
 * runs after the response lifecycle, and any failure is logged without
 * being propagated to the response.
 *
 * Tenant isolation: every audit entry is stamped with the acting user's
 * tenant (merchant) id so merchant-owned records stay scoped. Cross-tenant
 * admin access is only permitted for explicitly flagged admins, and the
 * bypass is recorded on the entry itself (audited) rather than silently
 * widening the query scope.
 */
function auditLog(action) {
  return (req, res, next) => {
    const user = req.user || null;
    const tenantId = user ? user.tenantId || user.merchantId || null : null;
    const isAdmin = Boolean(user && user.isAdmin);
    const requestedTenantId = req.params && req.params.tenantId
      ? req.params.tenantId
      : null;

    // An admin may act across tenants, but only when the target tenant is
    // explicitly provided. The bypass is captured on the audit entry so it
    // can be reviewed later.
    const crossTenantBypass = Boolean(
      isAdmin && requestedTenantId && requestedTenantId !== tenantId
    );

    const entry = {
      action,
      userId: user ? user.id : null,
      tenantId,
      targetTenantId: requestedTenantId,
      crossTenantBypass,
      method: req.method,
      path: req.originalUrl,
      ip: req.ip,
      createdAt: new Date(),
    };

    setImmediate(() => {
      prisma.auditLog
        .create({ data: entry })
        .catch((err) => {
          // console.error is the single allowed exception to the backend
          // `no-console` rule (see eslint.config.mjs) for top-level catch blocks.
          console.error('Failed to write audit log entry:', err);
        });
    });

    next();
  };
}

module.exports = auditLog;
module.exports.pseudonymizeAuditLogForUser = pseudonymizeAuditLogForUser;
module.exports.pseudonymize = pseudonymize;
