const crypto = require('crypto');
const request = require('supertest');

jest.mock('@stellar/stellar-sdk', () => ({
  Horizon: { Server: jest.fn() },
  StrKey: { isValidEd25519PublicKey: jest.fn(() => true) },
  Keypair: { fromPublicKey: jest.fn() },
}));

jest.mock('redis', () => ({
  createClient: jest.fn(() => null),
}));

jest.mock('../prismaClient', () => ({
  prisma: {
    user: {
      findUnique: jest.fn().mockResolvedValue(null),
      findFirst: jest.fn().mockResolvedValue(null),
    },
    $queryRaw: jest.fn().mockResolvedValue([{ '1': 1 }]),
    webhook: {
      create: jest.fn(),
      findMany: jest.fn(),
      deleteMany: jest.fn(),
    },
  },
  isPrismaConnectionError: jest.fn().mockReturnValue(false),
}));

process.env.NODE_ENV = 'test';

const { app } = require('../server');

describe('POST /api/v1/webhooks/verify-test', () => {
  test('accepts a payload and signature and returns success', async () => {
    const secret = 'test-webhook-secret';
    const payload = { event: 'payment.created', id: 'evt_123', amount: 42 };
    const signature = crypto.createHmac('sha256', secret).update(JSON.stringify(payload)).digest('hex');

    const res = await request(app)
      .post('/api/v1/webhooks/verify-test')
      .set('X-Webhook-Signature', signature)
      .send({ secret, payload: JSON.stringify(payload) });

    expect(res.status).toBe(200);
    expect(res.body.ok).toBe(true);
    expect(res.body.valid).toBe(true);
    expect(res.body.message).toMatch(/succeeded/i);
    expect(res.body.expectedSignature).toBe(signature);
  });

  test('returns detailed failure when the signature does not match', async () => {
    const secret = 'test-webhook-secret';
    const payload = { event: 'payment.created', id: 'evt_123', amount: 42 };

    const res = await request(app)
      .post('/api/v1/webhooks/verify-test')
      .set('X-Webhook-Signature', 'abc123')
      .send({ secret, payload: JSON.stringify(payload) });

    expect(res.status).toBe(401);
    expect(res.body.ok).toBe(false);
    expect(res.body.valid).toBe(false);
    expect(res.body.error).toMatchObject({
      code: 'INVALID_WEBHOOK_SIGNATURE',
    });
    expect(res.body.receivedSignature).toBe('abc123');
  });
});

describe('sendWebhook — timestamp-prefixed HMAC signature (issue #14)', () => {
  beforeEach(() => {
    global.fetch = jest.fn();
  });

  afterEach(() => {
    delete global.fetch;
  });

  test('sends X-Webhook-Timestamp header as a Unix timestamp (integer seconds)', async () => {
    global.fetch.mockResolvedValue({ ok: true });
    const { sendWebhook } = require('../src/webhookWorker');

    const before = Math.floor(Date.now() / 1000);
    await sendWebhook('https://merchant.example.com/hook', { event: 'payment.received' }, 'secret');
    const after = Math.floor(Date.now() / 1000);

    const [, opts] = global.fetch.mock.calls[0];
    const ts = Number(opts.headers['X-Webhook-Timestamp']);
    expect(Number.isInteger(ts)).toBe(true);
    expect(ts).toBeGreaterThanOrEqual(before);
    expect(ts).toBeLessThanOrEqual(after);
  });

  test('signature is computed over "${timestamp}.${rawBody}" not just the body', async () => {
    global.fetch.mockResolvedValue({ ok: true });
    const { sendWebhook } = require('../src/webhookWorker');

    const secret = 'mysecret';
    const payload = { event: 'payment.received', amount: '10.00' };
    await sendWebhook('https://merchant.example.com/hook', payload, secret);

    const [, opts] = global.fetch.mock.calls[0];
    const ts = opts.headers['X-Webhook-Timestamp'];
    const rawBody = JSON.stringify(payload);

    const expectedSig = crypto
      .createHmac('sha256', secret)
      .update(`${ts}.${rawBody}`)
      .digest('hex');

    expect(opts.headers['X-Webhook-Signature']).toBe(expectedSig);
    expect(opts.headers['X-Stellar-Tags-Signature']).toBe(expectedSig);
  });

  test('signature computed over raw body alone no longer matches (guards against regression)', async () => {
    global.fetch.mockResolvedValue({ ok: true });
    const { sendWebhook } = require('../src/webhookWorker');

    const secret = 'mysecret';
    const payload = { event: 'payment.received' };
    await sendWebhook('https://merchant.example.com/hook', payload, secret);

    const [, opts] = global.fetch.mock.calls[0];
    const rawBody = JSON.stringify(payload);

    const bodyOnlySig = crypto
      .createHmac('sha256', secret)
      .update(rawBody)
      .digest('hex');

    // The sent signature must NOT match a body-only HMAC (timestamp is required).
    expect(opts.headers['X-Webhook-Signature']).not.toBe(bodyOnlySig);
  });
});
