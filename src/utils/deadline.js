'use strict';

/**
 * Request-scoped deadline propagation (#363).
 *
 * Usage:
 *   const { withDeadline, DeadlineError } = require('./deadline');
 *
 *   // Wrap any async call. Rejects with DeadlineError when the signal fires.
 *   const result = await withDeadline(fetch(url), req.signal);
 */

class DeadlineError extends Error {
  constructor(message = 'Request deadline exceeded') {
    super(message);
    this.name = 'DeadlineError';
    this.code = 'DEADLINE_EXCEEDED';
    this.statusCode = 503;
  }
}

/**
 * Race a promise against an AbortSignal.
 * @param {Promise<T>} promise
 * @param {AbortSignal | null | undefined} signal
 * @returns {Promise<T>}
 */
function withDeadline(promise, signal) {
  if (!signal) return promise;
  if (signal.aborted) return Promise.reject(new DeadlineError());

  return new Promise((resolve, reject) => {
    function onAbort() {
      reject(new DeadlineError());
    }
    signal.addEventListener('abort', onAbort, { once: true });
    promise.then(
      (value) => {
        signal.removeEventListener('abort', onAbort);
        resolve(value);
      },
      (err) => {
        signal.removeEventListener('abort', onAbort);
        reject(err);
      }
    );
  });
}

/**
 * Express middleware: attaches an AbortController to `req` and aborts it when
 * the client disconnects or when REQUEST_DEADLINE_MS elapses (whichever first).
 *
 * Downstream code accesses the signal via `req.signal`.
 */
function deadlineMiddleware(req, res, next) {
  const deadlineMs = Number(process.env.REQUEST_DEADLINE_MS) || 0;
  const controller = new AbortController();
  req.signal = controller.signal;

  let timer = null;
  if (deadlineMs > 0) {
    timer = setTimeout(() => controller.abort(), deadlineMs);
  }

  function cleanup() {
    if (timer) clearTimeout(timer);
    controller.abort();
  }

  req.on('close', cleanup);
  res.on('finish', () => {
    req.off('close', cleanup);
    if (timer) clearTimeout(timer);
  });

  next();
}

module.exports = { withDeadline, DeadlineError, deadlineMiddleware };
