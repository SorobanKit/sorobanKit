'use strict';

const REQUIRED_ENV_VARS = ['PORT'];

for (const key of REQUIRED_ENV_VARS) {
  if (!process.env[key]) {
    if (process.env.NODE_ENV !== 'test') {
      console.error(`[env] Missing required environment variable: ${key}`);
      process.exit(1);
    }
  }
}

if (!process.env.REDIS_URL && process.env.NODE_ENV !== 'test') {
  console.warn('[env] Warning: REDIS_URL is not set. Features relying on Redis (rate limiting, idempotency cache, federation cache, webhook worker) will fall back to local defaults or fail.');
}

