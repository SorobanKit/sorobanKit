let logger = console;
try {
  logger = require('../logger').logger || console;
} catch (e) {
  try {
    logger = require('../../stellar-payment-platform/src/logger').logger || console;
  } catch (err) {
    logger = console;
  }
}

let defaultDeprecations = [];
try {
  defaultDeprecations = require('../config/deprecations').DEPRECATIONS || [];
} catch (e) {
  defaultDeprecations = [];
}

/**
 * Formats an ISO-8601 date as an RFC 7231 HTTP-date.
 *
 * @param {string|Date} dateVal
 * @returns {string|null}
 */
function toHttpDate(dateVal) {
  if (!dateVal) return null;
  const d = new Date(dateVal);
  if (Number.isNaN(d.getTime())) return null;
  return d.toUTCString();
}

const escapeRegExp = (value) => value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

/**
 * Matches request path against a pattern (supports trailing wildcard '*').
 *
 * @param {string} pattern
 * @param {string} path
 * @returns {boolean}
 */
function pathMatches(pattern, path) {
  if (pattern === path) return true;
  if (pattern.includes('*')) {
    const regex = new RegExp(`^${pattern.split('*').map(escapeRegExp).join('.*')}$`);
    return regex.test(path);
  }
  return false;
}

/**
 * Finds a matching deprecation entry by method and path.
 *
 * @param {string} method
 * @param {string} path
 * @param {Array} [registry]
 * @returns {object|null}
 */
function findDeprecation(method, path, registry = defaultDeprecations) {
  const verb = String(method || '').toUpperCase();
  for (const entry of registry) {
    const entryMethod = String(entry.method || '*').toUpperCase();
    if (entryMethod !== '*' && entryMethod !== verb) continue;
    if (pathMatches(entry.path, path)) return entry;
  }
  return null;
}

/**
 * Middleware that marks deprecated endpoints by setting the standard
 * `Deprecation` and `Sunset` response headers per RFC 8594.
 *
 * @param {Object} [options]
 * @param {string} [options.sunset] - Direct HTTP date string or ISO date override.
 * @param {string} [options.link] - Link header value pointing to deprecation docs.
 * @param {Array} [options.registry] - Deprecation registry to match against.
 * @returns {import('express').RequestHandler}
 */
function deprecation({ sunset, link, registry = defaultDeprecations } = {}) {
  return function deprecationMiddleware(req, res, next) {
    const path = req.path || (req.url ? req.url.split('?')[0] : '');
    const entry = findDeprecation(req.method, path, registry);

    if (entry || sunset || link) {
      const deprecationDate = entry && entry.deprecatedSince
        ? toHttpDate(entry.deprecatedSince)
        : 'true';
      res.setHeader('Deprecation', deprecationDate || 'true');

      const sunsetDateRaw = (entry && entry.sunsetDate) || sunset;
      const sunsetHttpDate = toHttpDate(sunsetDateRaw);
      if (sunsetHttpDate) {
        res.setHeader('Sunset', sunsetHttpDate);
      }

      const linkHeader = entry && entry.documentation
        ? `<${entry.documentation}>; rel="deprecation"`
        : link;
      if (linkHeader) {
        res.setHeader('Link', linkHeader);
      }

      logger.warn('Deprecated endpoint called', {
        route: req.originalUrl || req.url,
        method: req.method,
        sunset: sunsetHttpDate || null,
        correlationId: req.correlationId || req.id,
      });
    }

    next();
  };
}

module.exports = deprecation;
module.exports.deprecation = deprecation;
module.exports.toHttpDate = toHttpDate;
module.exports.findDeprecation = findDeprecation;
module.exports.pathMatches = pathMatches;
