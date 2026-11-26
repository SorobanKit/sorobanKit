'use strict';

/**
 * Per-network circuit breaker for Stellar/Horizon upstreams (#364).
 *
 * States: CLOSED → OPEN (on threshold failures) → HALF_OPEN → CLOSED (on probe success).
 *
 * Configuration via environment variables (all optional):
 *   CB_FAILURE_THRESHOLD      – consecutive failures before opening (default 5)
 *   CB_RECOVERY_TIMEOUT_MS    – ms before moving from OPEN to HALF_OPEN (default 30000)
 *   CB_SUCCESS_THRESHOLD      – consecutive successes in HALF_OPEN to close (default 2)
 */

const STATES = Object.freeze({ CLOSED: 'CLOSED', OPEN: 'OPEN', HALF_OPEN: 'HALF_OPEN' });

class CircuitBreakerOpenError extends Error {
  constructor(network) {
    super(`Circuit breaker open for network: ${network}`);
    this.name = 'CircuitBreakerOpenError';
    this.code = 'CIRCUIT_OPEN';
    this.statusCode = 503;
    this.network = network;
  }
}

class CircuitBreaker {
  /**
   * @param {string} network  - opaque label shown in errors/metrics
   * @param {{ failureThreshold?: number, recoveryTimeoutMs?: number, successThreshold?: number }} [opts]
   */
  constructor(network, opts = {}) {
    this.network = network;
    this.failureThreshold = opts.failureThreshold ?? Number(process.env.CB_FAILURE_THRESHOLD) || 5;
    this.recoveryTimeoutMs = opts.recoveryTimeoutMs ?? Number(process.env.CB_RECOVERY_TIMEOUT_MS) || 30_000;
    this.successThreshold = opts.successThreshold ?? Number(process.env.CB_SUCCESS_THRESHOLD) || 2;

    this._state = STATES.CLOSED;
    this._failures = 0;
    this._successes = 0;
    this._openedAt = null;
  }

  get state() {
    return this._state;
  }

  _maybeTransitionFromOpen() {
    if (this._state === STATES.OPEN && Date.now() - this._openedAt >= this.recoveryTimeoutMs) {
      this._state = STATES.HALF_OPEN;
      this._successes = 0;
    }
  }

  _onSuccess() {
    if (this._state === STATES.HALF_OPEN) {
      this._successes += 1;
      if (this._successes >= this.successThreshold) {
        this._state = STATES.CLOSED;
        this._failures = 0;
        this._openedAt = null;
      }
    } else {
      this._failures = 0;
    }
  }

  _onFailure() {
    this._failures += 1;
    this._successes = 0;
    if (this._state === STATES.HALF_OPEN || this._failures >= this.failureThreshold) {
      this._state = STATES.OPEN;
      this._openedAt = Date.now();
    }
  }

  /**
   * Execute fn through the breaker.
   * @param {() => Promise<T>} fn
   * @returns {Promise<T>}
   */
  async execute(fn) {
    this._maybeTransitionFromOpen();

    if (this._state === STATES.OPEN) {
      throw new CircuitBreakerOpenError(this.network);
    }

    try {
      const result = await fn();
      this._onSuccess();
      return result;
    } catch (err) {
      this._onFailure();
      throw err;
    }
  }
}

/** Singleton registry — one breaker per network label. */
const _registry = new Map();

function getBreaker(network, opts) {
  if (!_registry.has(network)) {
    _registry.set(network, new CircuitBreaker(network, opts));
  }
  return _registry.get(network);
}

/** For testing: reset all breakers. */
function resetBreakers() {
  _registry.clear();
}

module.exports = { CircuitBreaker, CircuitBreakerOpenError, getBreaker, resetBreakers, STATES };
