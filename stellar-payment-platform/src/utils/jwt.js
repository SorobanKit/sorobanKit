'use strict';

/**
 * JWT utility — RS256 (asymmetric) signing and verification.
 *
 * Keys are loaded from environment variables:
 *   JWT_PRIVATE_KEY  — PEM-encoded RSA private key (for signing)
 *   JWT_PUBLIC_KEY   — PEM-encoded RSA public key  (for verification / JWKS)
 *
 * Generate a key pair locally with:
 *   openssl genrsa -out private.pem 2048
 *   openssl rsa -in private.pem -pubout -out public.pem
 * Then set the env vars to the file contents (newlines preserved).
 */

const jwt = require('jsonwebtoken');
const crypto = require('crypto');

// ---------------------------------------------------------------------------
// Key loading
// ---------------------------------------------------------------------------

/**
 * Replace literal "\n" escape sequences with real newlines so the key works
 * whether it was stored as a single-line env var or a multi-line one.
 */
const normalizeKey = (raw) =>
  typeof raw === 'string' ? raw.replace(/\\n/g, '\n') : raw;

const PRIVATE_KEY = normalizeKey(process.env.JWT_PRIVATE_KEY);
const PUBLIC_KEY = normalizeKey(process.env.JWT_PUBLIC_KEY);

const DEFAULT_EXPIRY = process.env.JWT_EXPIRY || '1h';
const DEFAULT_ISSUER = 'stellar-payment-platform';
const DEFAULT_AUDIENCE = 'api';

// ---------------------------------------------------------------------------
// Sign
// ---------------------------------------------------------------------------

/**
 * Sign a payload with the RSA private key using RS256.
 *
 * @param {object} payload  - Claims to embed in the token.
 * @param {object} [options] - Optional jsonwebtoken sign options (e.g. expiresIn).
 * @returns {string} Signed JWT string.
 */
function signToken(payload, options = {}) {
  if (!PRIVATE_KEY) {
    throw new Error('JWT_PRIVATE_KEY is not configured.');
  }

  return jwt.sign(payload, PRIVATE_KEY, {
    algorithm: 'RS256',
    expiresIn: DEFAULT_EXPIRY,
    issuer: DEFAULT_ISSUER,
    audience: DEFAULT_AUDIENCE,
    ...options,
  });
}

// ---------------------------------------------------------------------------
// Verify
// ---------------------------------------------------------------------------

/**
 * Verify and decode a JWT using the RSA public key.
 *
 * @param {string} token - JWT string to verify.
 * @param {object} [options] - Optional jsonwebtoken verify options.
 * @returns {object} Decoded payload.
 * @throws {JsonWebTokenError|TokenExpiredError} on invalid / expired tokens.
 */
function verifyToken(token, options = {}) {
  if (!PUBLIC_KEY) {
    throw new Error('JWT_PUBLIC_KEY is not configured.');
  }

  try {
    return jwt.verify(token, PUBLIC_KEY, {
      algorithms: ['RS256'],
      issuer: DEFAULT_ISSUER,
      audience: DEFAULT_AUDIENCE,
      ...options,
    });
  } catch (err) {
    if (
      err.name === 'JsonWebTokenError' &&
      (err.message.includes('jwt issuer invalid') ||
        err.message.includes('jwt audience invalid') ||
        err.message.includes('issuer') ||
        err.message.includes('audience'))
    ) {
      const authErr = new Error('UNAUTHENTICATED');
      authErr.name = 'JsonWebTokenError';
      authErr.code = 'UNAUTHENTICATED';
      throw authErr;
    }
    throw err;
  }
}

// ---------------------------------------------------------------------------
// JWKS helpers
// ---------------------------------------------------------------------------

/**
 * Build a JSON Web Key Set (JWKS) document from the configured public key.
 * The key ID (kid) is derived from the SHA-256 thumbprint of the DER-encoded
 * public key so it stays stable across restarts.
 *
 * @returns {{ keys: object[] }} JWKS document.
 */
function getJwks() {
  if (!PUBLIC_KEY) {
    throw new Error('JWT_PUBLIC_KEY is not configured.');
  }

  const keyObject = crypto.createPublicKey(PUBLIC_KEY);
  const jwk = keyObject.export({ format: 'jwk' });

  // Compute a stable key ID: SHA-256 thumbprint of the JWK (RFC 7638)
  const thumbprintInput = JSON.stringify({
    e: jwk.e,
    kty: jwk.kty,
    n: jwk.n,
  });
  const kid = crypto.createHash('sha256').update(thumbprintInput).digest('base64url');

  return {
    keys: [
      {
        kty: jwk.kty,     // "RSA"
        use: 'sig',
        alg: 'RS256',
        kid,
        n: jwk.n,
        e: jwk.e,
      },
    ],
  };
}

/**
 * Express middleware: extract and verify a Bearer JWT from the Authorization
 * header, attaching the decoded payload to `req.user`.
 */
function requireAuth(req, res, next) {
  const authHeader = req.headers.authorization || '';
  const [scheme, token] = authHeader.split(' ');

  if (scheme !== 'Bearer' || !token) {
    return res.status(401).json({ error: 'Missing or malformed Authorization header.' });
  }

  try {
    req.user = verifyToken(token);
    return next();
  } catch (err) {
    if (err.message === 'UNAUTHENTICATED' || err.code === 'UNAUTHENTICATED') {
      return res.status(401).json({ error: 'UNAUTHENTICATED' });
    }
    const message =
      err.name === 'TokenExpiredError' ? 'Token has expired.' : 'Invalid token.';
    return res.status(401).json({ error: message });
  }
}

module.exports = {
  signToken,
  verifyToken,
  getJwks,
  requireAuth,
  DEFAULT_ISSUER,
  DEFAULT_AUDIENCE,
};
