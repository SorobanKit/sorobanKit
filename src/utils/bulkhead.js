'use strict';

/**
 * Per-network concurrency bulkhead for Stellar/Horizon upstreams (#365).
 *
 * Limits how many calls to a given network are in-flight at once and how deep
 * the wait queue can grow. A slow network can neither exhaust process resources
 * nor starve requests bound for a healthy network.
 *
 * Configuration via environment variables (all optional):
 *   BULKHEAD_CONCURRENCY   – max simultaneous in-flight calls per network (default 10)
 *   BULKHEAD_QUEUE_DEPTH   – max queued callers (default 20)
 *   BULKHEAD_QUEUE_TIMEOUT_MS – ms a queued call may wait before rejection (default 5000)
 */

class BulkheadRejectedError extends Error {
  constructor(network, reason = 'queue full') {
    super(`Bulkhead rejected for network "${network}": ${reason}`);
    this.name = 'BulkheadRejectedError';
    this.code = 'BULKHEAD_REJECTED';
    this.statusCode = 503;
    this.network = network;
  }
}

class Bulkhead {
  /**
   * @param {string} network
   * @param {{ concurrency?: number, queueDepth?: number, queueTimeoutMs?: number }} [opts]
   */
  constructor(network, opts = {}) {
    this.network = network;
    this.concurrency = opts.concurrency ?? Number(process.env.BULKHEAD_CONCURRENCY) || 10;
    this.queueDepth = opts.queueDepth ?? Number(process.env.BULKHEAD_QUEUE_DEPTH) || 20;
    this.queueTimeoutMs = opts.queueTimeoutMs ?? Number(process.env.BULKHEAD_QUEUE_TIMEOUT_MS) || 5_000;

    this._active = 0;
    this._queue = [];
  }

  get activeCount() { return this._active; }
  get queueLength() { return this._queue.length; }

  _tryDequeue() {
    while (this._active < this.concurrency && this._queue.length > 0) {
      const { resolve } = this._queue.shift();
      this._active += 1;
      resolve();
    }
  }

  _release() {
    this._active -= 1;
    this._tryDequeue();
  }

  /**
   * Execute fn through the bulkhead.
   * @param {() => Promise<T>} fn
   * @returns {Promise<T>}
   */
  async execute(fn) {
    if (this._active < this.concurrency) {
      this._active += 1;
    } else if (this._queue.length >= this.queueDepth) {
      throw new BulkheadRejectedError(this.network, 'queue full');
    } else {
      await new Promise((resolve, reject) => {
        let timer = null;
        const entry = { resolve, reject };
        if (this.queueTimeoutMs > 0) {
          timer = setTimeout(() => {
            const idx = this._queue.indexOf(entry);
            if (idx !== -1) this._queue.splice(idx, 1);
            reject(new BulkheadRejectedError(this.network, 'queue timeout'));
          }, this.queueTimeoutMs);
        }
        const wrappedResolve = () => {
          if (timer) clearTimeout(timer);
          resolve();
        };
        this._queue.push({ resolve: wrappedResolve, reject });
      });
    }

    try {
      return await fn();
    } finally {
      this._release();
    }
  }
}

/** Singleton registry — one bulkhead per network label. */
const _registry = new Map();

function getBulkhead(network, opts) {
  if (!_registry.has(network)) {
    _registry.set(network, new Bulkhead(network, opts));
  }
  return _registry.get(network);
}

/** For testing: reset all bulkheads. */
function resetBulkheads() {
  _registry.clear();
}

module.exports = { Bulkhead, BulkheadRejectedError, getBulkhead, resetBulkheads };
