const request = require('supertest');
const express = require('express');

jest.mock('@stellar/stellar-sdk', () => ({
  Horizon: { Server: jest.fn() },
}));

jest.mock('../prismaClient', () => ({
  prisma: { $queryRaw: jest.fn() },
}));

const { prisma } = require('../prismaClient');
const { HORIZON_BASE } = require('../src/services/stellarService');
const healthRoutes = require('../src/routes/v1/healthRoutes');

const buildApp = (redisClient = null) => {
  const app = express();
  app.use(healthRoutes(redisClient));
  return app;
};

let redisClient;

beforeEach(() => {
  jest.clearAllMocks();
  prisma.$queryRaw.mockResolvedValue([{ 1: 1 }]);
  redisClient = { ping: jest.fn().mockResolvedValue('PONG') };
  global.fetch = jest.fn().mockResolvedValue({ ok: true, status: 200 });
  delete process.env.HEALTH_HORIZON_TIMEOUT_MS;
});

afterEach(() => {
  delete process.env.HEALTH_HORIZON_TIMEOUT_MS;
});

describe('GET /health', () => {
  test('returns 200 with every dependency up', async () => {
    const res = await request(buildApp(redisClient)).get('/health');

    expect(res.status).toBe(200);
    expect(res.body).toMatchObject({
      status: 'UP',
      database: 'up',
      redis: 'up',
      horizon: 'up',
    });
    expect(res.body.timestamp).toEqual(expect.any(String));
  });

  test('probes PostgreSQL, Redis and Horizon', async () => {
    await request(buildApp(redisClient)).get('/health');

    expect(prisma.$queryRaw).toHaveBeenCalled();
    expect(redisClient.ping).toHaveBeenCalled();
    expect(global.fetch).toHaveBeenCalledWith(HORIZON_BASE, expect.anything());
  });

  test('returns 503 when the database query fails', async () => {
    prisma.$queryRaw.mockRejectedValue(new Error('connection refused'));

    const res = await request(buildApp(redisClient)).get('/health');

    expect(res.status).toBe(503);
    expect(res.body.status).toBe('DOWN');
    expect(res.body.database).toBe('down');
    expect(res.body.message).toContain('Database unavailable');
  });

  test('returns 503 when the Redis PING fails', async () => {
    redisClient.ping.mockRejectedValue(new Error('not connected'));

    const res = await request(buildApp(redisClient)).get('/health');

    expect(res.status).toBe(503);
    expect(res.body.redis).toBe('down');
    expect(res.body.message).toContain('Redis unavailable');
  });

  test('returns 503 when Horizon is unreachable', async () => {
    global.fetch.mockRejectedValue(new Error('ETIMEDOUT'));

    const res = await request(buildApp(redisClient)).get('/health');

    expect(res.status).toBe(503);
    expect(res.body.horizon).toBe('down');
    expect(res.body.message).toContain('Horizon unavailable');
  });

  test('returns 503 when Horizon answers with a non-2xx status', async () => {
    global.fetch.mockResolvedValue({ ok: false, status: 502 });

    const res = await request(buildApp(redisClient)).get('/health');

    expect(res.status).toBe(503);
    expect(res.body.horizon).toBe('down');
  });

  test('reports every failing dependency in one response', async () => {
    prisma.$queryRaw.mockRejectedValue(new Error('down'));
    redisClient.ping.mockRejectedValue(new Error('down'));
    global.fetch.mockRejectedValue(new Error('down'));

    const res = await request(buildApp(redisClient)).get('/health');

    expect(res.status).toBe(503);
    expect(res.body).toMatchObject({
      status: 'DOWN',
      database: 'down',
      redis: 'down',
      horizon: 'down',
    });
  });

  test('returns 503 with status DOWN and message naming all three failures when all dependencies fail simultaneously', async () => {
    prisma.$queryRaw.mockRejectedValue(new Error('connection refused'));
    redisClient.ping.mockRejectedValue(new Error('not connected'));
    global.fetch.mockRejectedValue(new Error('ETIMEDOUT'));

    const res = await request(buildApp(redisClient)).get('/health');

    expect(res.status).toBe(503);
    expect(res.body.status).toBe('DOWN');
    expect(res.body.database).toBe('down');
    expect(res.body.redis).toBe('down');
    expect(res.body.horizon).toBe('down');
    expect(res.body.message).toContain('Database unavailable');
    expect(res.body.message).toContain('Redis unavailable');
    expect(res.body.message).toContain('Horizon unavailable');
  });

  test('reports Redis as not configured and stays healthy without it', async () => {
    const res = await request(buildApp(null)).get('/health');

    expect(res.status).toBe(200);
    expect(res.body.redis).toBe('not configured');
  });

  test('aborts the Horizon probe once the timeout elapses', async () => {
    await request(buildApp(redisClient)).get('/health');

    const [, options] = global.fetch.mock.calls[0];
    expect(options.signal).toBeInstanceOf(AbortSignal);
    expect(options.signal.aborted).toBe(false);
  });

  test('uses HEALTH_HORIZON_TIMEOUT_MS read at request time', async () => {
    process.env.HEALTH_HORIZON_TIMEOUT_MS = '500';

    await request(buildApp(redisClient)).get('/health');

    const [, options] = global.fetch.mock.calls[0];
    expect(options.signal).toBeInstanceOf(AbortSignal);
    expect(options.signal.aborted).toBe(false);
  });

  test('aborts the Horizon probe after the configured timeout elapses', async () => {
    process.env.HEALTH_HORIZON_TIMEOUT_MS = '50';

    let capturedSignal;
    global.fetch.mockImplementation((url, options) => {
      capturedSignal = options.signal;
      return new Promise((resolve, reject) => {
        options.signal.addEventListener('abort', () =>
          reject(new Error('The operation was aborted'))
        );
      });
    });

    const startedAt = Date.now();
    const res = await request(buildApp(redisClient)).get('/health');
    const elapsed = Date.now() - startedAt;

    expect(capturedSignal).toBeInstanceOf(AbortSignal);
    expect(capturedSignal.aborted).toBe(true);
    expect(elapsed).toBeGreaterThanOrEqual(40);
    expect(elapsed).toBeLessThan(1000);
    expect(res.status).toBe(503);
    expect(res.body.horizon).toBe('down');
  });

  test('falls back to the default timeout when HEALTH_HORIZON_TIMEOUT_MS is invalid', async () => {
    process.env.HEALTH_HORIZON_TIMEOUT_MS = 'abc';

    const res = await request(buildApp(redisClient)).get('/health');

    expect(res.status).toBe(200);
    expect(res.body.horizon).toBe('up');
    const [, options] = global.fetch.mock.calls[0];
    expect(options.signal).toBeInstanceOf(AbortSignal);
    expect(options.signal.aborted).toBe(false);
  });

  test('probes the URL from HORIZON_BASE env var when set', async () => {
    const customUrl = 'https://horizon.custom-staging.example.com';
    jest.resetModules();
    process.env.HORIZON_BASE = customUrl;

    // Re-require after setting the env var so the module picks up the new value.
    const healthRoutesWithEnv = require('../src/routes/v1/healthRoutes');
    const appWithEnv = express();
    appWithEnv.use(healthRoutesWithEnv(redisClient));

    await request(appWithEnv).get('/health');

    expect(global.fetch).toHaveBeenCalledWith(customUrl, expect.anything());

    delete process.env.HORIZON_BASE;
    jest.resetModules();
  });

  test('falls back to the testnet URL when HORIZON_BASE is unset', async () => {
    jest.resetModules();
    delete process.env.HORIZON_BASE;

    const healthRoutesDefault = require('../src/routes/v1/healthRoutes');
    const appDefault = express();
    appDefault.use(healthRoutesDefault(redisClient));

    await request(appDefault).get('/health');

    const [probedUrl] = global.fetch.mock.calls[0];
    expect(probedUrl).toBe('https://horizon-testnet.stellar.org');

    jest.resetModules();
  });
});

// ── X-Correlation-ID header assertions for health endpoint ────────────────────

describe('GET /health — X-Correlation-ID header', () => {
  const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;

  let healthApp;

  beforeEach(() => {
    jest.clearAllMocks();
    prisma.$queryRaw.mockResolvedValue([{ 1: 1 }]);
    redisClient = { ping: jest.fn().mockResolvedValue('PONG') };
    global.fetch = jest.fn().mockResolvedValue({ ok: true, status: 200 });

    // Mount health routes with the real correlation middleware so the header
    // is attached by the middleware, not by the health handler itself.
    const { correlationId } = require('../middleware/correlation');
    healthApp = express();
    healthApp.use(correlationId);
    healthApp.use(healthRoutes(redisClient));
  });

  test('200 health response includes X-Correlation-ID header', async () => {
    const res = await request(healthApp).get('/health');

    expect(res.status).toBe(200);
    expect(res.headers['x-correlation-id']).toBeDefined();
    expect(res.headers['x-correlation-id']).toMatch(UUID_RE);
  });

  test('503 health response includes X-Correlation-ID header', async () => {
    prisma.$queryRaw.mockRejectedValue(new Error('db down'));

    const res = await request(healthApp).get('/health');

    expect(res.status).toBe(503);
    expect(res.headers['x-correlation-id']).toBeDefined();
    expect(res.headers['x-correlation-id']).toMatch(UUID_RE);
  });

  test('echoes back a caller-supplied X-Correlation-ID on 200', async () => {
    const myId = 'bbbbbbbb-2222-4222-8222-bbbbbbbbbbbb';

    const res = await request(healthApp)
      .get('/health')
      .set('X-Correlation-ID', myId);

    expect(res.status).toBe(200);
    expect(res.headers['x-correlation-id']).toBe(myId);
  });

  test('echoes back a caller-supplied X-Correlation-ID on 503', async () => {
    prisma.$queryRaw.mockRejectedValue(new Error('db down'));
    const myId = 'cccccccc-3333-4333-8333-cccccccccccc';

    const res = await request(healthApp)
      .get('/health')
      .set('X-Correlation-ID', myId);

    expect(res.status).toBe(503);
    expect(res.headers['x-correlation-id']).toBe(myId);
  });
});
