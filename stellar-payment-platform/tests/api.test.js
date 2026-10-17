const request = require('supertest');

// Mock the Stellar SDK to prevent Jest ESM syntax errors
jest.mock('@stellar/stellar-sdk', () => ({
  Horizon: { Server: jest.fn() },
  StrKey: { isValidEd25519PublicKey: jest.fn(() => true) }
}));

jest.mock('redis', () => ({
  createClient: jest.fn(() => null),
}));

// Mock Prisma so it doesn't try to connect to a real database and crash
jest.mock('../prismaClient', () => ({
  prisma: {
    user: {
      findUnique: jest.fn().mockResolvedValue(null),
      findFirst: jest.fn().mockResolvedValue(null),
      create: jest.fn().mockResolvedValue({ id: 'mock-id', username: 'newuser', address: 'GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF' })
    },
    $queryRaw: jest.fn().mockResolvedValue([{ '1': 1 }]),
  },
  isPrismaConnectionError: jest.fn().mockReturnValue(false),
}));

process.env.NODE_ENV = 'test';
const { app } = require('../server');

describe('Full Middleware Chain & API Contract Integration Tests', () => {
  describe('Correlation ID Middleware', () => {
    it('generates a new correlation ID if x-correlation-id header is omitted', async () => {
      const res = await request(app).get('/health');
      expect(res.headers['x-correlation-id']).toBeDefined();
      expect(typeof res.headers['x-correlation-id']).toBe('string');
      expect(res.headers['x-correlation-id'].length).toBeGreaterThan(0);
    });

    it('propagates the client-supplied x-correlation-id header through the response', async () => {
      const customId = 'test-trace-id-abc-12345';
      const res = await request(app)
        .get('/health')
        .set('x-correlation-id', customId);
      expect(res.headers['x-correlation-id']).toBe(customId);
    });
  });

  describe('Rate Limiting & Security Headers', () => {
    it('injects rate-limiting headers (standard or legacy) on API requests', async () => {
      const res = await request(app)
        .get('/api/v1/federation')
        .query({ q: 'client*localhost' });

      expect(
        res.headers['ratelimit-limit'] || res.headers['x-ratelimit-limit']
      ).toBeDefined();
      expect(
        res.headers['ratelimit-remaining'] || res.headers['x-ratelimit-remaining']
      ).toBeDefined();
    });

    it('injects strict security headers from security middleware', async () => {
      const res = await request(app).get('/health');
      expect(res.headers['x-content-type-options']).toBe('nosniff');
      expect(res.headers['x-frame-options']).toBe('DENY');
      expect(res.headers['permissions-policy']).toBeDefined();
    });
  });

  describe('Request Body Parsing & Body Limit Validation', () => {
    it('properly parses valid JSON bodies and passes through to registration validation', async () => {
      const res = await request(app)
        .post('/api/v1/register')
        .send({
          username: 'validuser',
          address: 'GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF'
        })
        .set('Content-Type', 'application/json');

      expect(res.statusCode).not.toBe(413); // not oversized
      expect(res.statusCode).not.toBe(415); // valid content-type
      expect(res.headers['x-correlation-id']).toBeDefined();
    });

    it('rejects malformed JSON payloads with 400 Bad Request', async () => {
      const res = await request(app)
        .post('/api/v1/register')
        .set('Content-Type', 'application/json')
        .send('{"username": "broken", invalid_json}');

      expect(res.statusCode).toBe(400);
      expect(res.body).toHaveProperty('error');
    });
  });

  describe('Nested Objects Rejection Middleware (rejectNestedObjects)', () => {
    it('rejects nested objects in query parameters with 400 and INVALID_INPUT code', async () => {
      const res = await request(app)
        .get('/api/v1/federation')
        .query({ q: { nested: 'payload' } });

      expect(res.statusCode).toBe(400);
      expect(res.body.error).toHaveProperty('code', 'INVALID_INPUT');
      expect(res.body.error.message).toMatch(/nested objects and arrays are not allowed/i);
    });

    it('rejects nested objects in JSON body with 400 and INVALID_INPUT code', async () => {
      const res = await request(app)
        .post('/api/v1/register')
        .send({
          username: 'alice',
          address: 'GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF',
          nested: { exploit: true }
        });

      expect(res.statusCode).toBe(400);
      expect(res.body.error).toHaveProperty('code', 'INVALID_INPUT');
    });
  });

  describe('Schema Validation & Federation API Contracts', () => {
    it('rejects registration requests missing required fields with schema validation error', async () => {
      const res = await request(app)
        .post('/api/v1/register')
        .send({ username: 'missing_address' });

      expect(res.statusCode).toBe(422);
      expect(res.body).toHaveProperty('error');
    });

    it('returns 200 with federation shape for a known seeded user', async () => {
      const res = await request(app)
        .get('/api/v1/federation')
        .query({ q: 'client*localhost' });

      expect(res.statusCode).toBe(200);
      expect(res.body).toHaveProperty('stellar_address');
      expect(res.body).toHaveProperty('account_id');
      expect(res.headers['x-correlation-id']).toBeDefined();
    });

    it('returns 404 with error payload for an unknown federation query', async () => {
      const res = await request(app)
        .get('/api/v1/federation')
        .query({ q: 'nonexistentuser*localhost' });

      expect(res.statusCode).toBe(404);
      expect(res.body).toHaveProperty('error');
    });
  });

  describe('ETag & 404 Error Handling Chain', () => {
    it('returns 304 Not Modified when client provides matching If-None-Match ETag header', async () => {
      const firstRes = await request(app)
        .get('/api/v1/federation')
        .query({ q: 'client*localhost' });

      expect(firstRes.statusCode).toBe(200);
      const etag = firstRes.headers['etag'];
      if (etag) {
        const cachedRes = await request(app)
          .get('/api/v1/federation')
          .query({ q: 'client*localhost' })
          .set('If-None-Match', etag);

        expect(cachedRes.statusCode).toBe(304);
      }
    });

    it('returns formatted 404 error through notFoundHandler for non-existent routes', async () => {
      const res = await request(app).get('/api/v1/non-existent-route-404');
      expect(res.statusCode).toBe(404);
      expect(res.body).toHaveProperty('error');
      expect(res.headers['x-correlation-id']).toBeDefined();
    });
  });
});
