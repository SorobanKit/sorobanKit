if (!process.env.DATABASE_URL) {
  console.error('DATABASE_URL is not set — see README for database setup');
  throw new Error('DATABASE_URL is not set — see README for database setup');
}

const { PrismaClient } = require('@prisma/client');

// Reuse a single PrismaClient instance across the process.
// In test environments Jest clears the module cache between test files,
// which would otherwise create a new client (and new DB connections) on
// every require(). Storing the instance on globalThis prevents that.
const globalForPrisma = globalThis;

const prisma =
  globalForPrisma.__prisma ||
  new PrismaClient({
    log: process.env.NODE_ENV === 'development' ? ['query', 'error', 'warn'] : ['error'],
  });

if (process.env.NODE_ENV !== 'production') {
  globalForPrisma.__prisma = prisma;
}

module.exports = prisma;
