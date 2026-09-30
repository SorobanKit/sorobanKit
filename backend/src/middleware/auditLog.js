const { PrismaClient } = require('@prisma/client');

const prisma = new PrismaClient();

/**
 * Middleware that records admin actions in the AuditLog table.
 *
 * The insert is fire-and-forget: it is scheduled with setImmediate so it
 * runs after the response lifecycle, and any failure is logged without
 * being propagated to the response.
 */
function auditLog(action) {
  return (req, res, next) => {
    const entry = {
      action,
      userId: req.user ? req.user.id : null,
      method: req.method,
      path: req.originalUrl,
      ip: req.ip,
      createdAt: new Date(),
    };

    setImmediate(() => {
      prisma.auditLog
        .create({ data: entry })
        .catch((err) => {
          console.error('Failed to write audit log entry:', err);
        });
    });

    next();
  };
}

module.exports = auditLog;
