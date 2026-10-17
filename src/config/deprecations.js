'use strict';

/**
 * Deprecation registry (RFC 8594).
 *
 * Each entry describes an endpoint that is scheduled for removal. The
 * deprecation middleware reads this list and attaches `Deprecation`,
 * `Sunset` and `Link` headers to matching requests.
 */
const DEPRECATIONS = [
  {
    method: 'GET',
    path: '/api/v1/lookup',
    deprecatedSince: '2026-08-29',
    sunsetDate: '2027-02-28',
    replacement: '/api/v2/lookup',
    documentation: 'https://docs.stellar-tags.example/deprecations/lookup',
  },
  {
    method: 'GET',
    path: '/api/v1/stats',
    deprecatedSince: '2026-08-29',
    sunsetDate: '2027-01-31',
    replacement: '/api/v2/stats',
    documentation: 'https://docs.stellar-tags.example/deprecations/stats',
  },
  {
    method: 'POST',
    path: '/api/v1/payments/bulk',
    deprecatedSince: '2026-08-29',
    sunsetDate: '2027-03-31',
    replacement: '/api/v2/payments/bulk',
    documentation: 'https://docs.stellar-tags.example/deprecations/bulk-payments',
  },
];

module.exports = { DEPRECATIONS };
