'use strict';

const REQUIRED_ENV_VARS = ['PORT', 'DATABASE_URL', 'CONTRACT_ID'];

for (const key of REQUIRED_ENV_VARS) {
  if (!process.env[key]) {
    if (process.env.NODE_ENV !== 'test') {
      console.error(`[env] Missing required environment variable: ${key}`);
      process.exit(1);
    }
  }
}

module.exports = { REQUIRED_ENV_VARS };
