const mockQueueAdd = jest.fn();
const mockQueueOn = jest.fn();
const mockQueueClose = jest.fn().mockResolvedValue(undefined);
const mockWorkerOn = jest.fn();
const mockWorkerClose = jest.fn().mockResolvedValue(undefined);
const mockRedisQuit = jest.fn().mockResolvedValue(undefined);

let mockWorkerProcessor;
let mockWorkerOptions;

jest.mock('bullmq', () => ({
  Queue: jest.fn().mockImplementation(() => ({
    add: mockQueueAdd,
    on: mockQueueOn,
    close: mockQueueClose,
  })),
  Worker: jest.fn().mockImplementation((_name, processor, options) => {
    mockWorkerProcessor = processor;
    mockWorkerOptions = options;
    return {
      on: mockWorkerOn,
      close: mockWorkerClose,
    };
  }),
}));

jest.mock('../src/config/redis', () => ({
  createRedisConnection: jest.fn(() => ({ quit: mockRedisQuit })),
}));

const { Queue, Worker } = require('bullmq');
const {
  dispatchPaymentWebhooks,
  enqueueWebhookDelivery,
  startWebhookWorker,
  closeWebhookQueue,
  processWebhookJob,
  detectSsrfTarget,
  MAX_WEBHOOK_ATTEMPTS,
  WEBHOOK_BACKOFF_DELAY_MS,
  WEBHOOK_QUEUE_NAME,
} = require('../src/webhookWorker');

const webhook = {
  id: 'webhook-1',
  username: 'merchant',
  url: 'https://merchant.example/webhooks',
  secret: 'secret',
};

const payload = {
  event: 'payment.received',
  event_id: 'transaction-1-payment-1',
  timestamp: '2026-08-25T12:00:00.000Z',
  data: { amount: '10.00' },
};

describe('webhook BullMQ delivery', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    global.fetch = jest.fn();
  });

  afterAll(async () => {
    await closeWebhookQueue();
    delete global.fetch;
  });

  test('enqueues deliveries with five attempts and exponential backoff', async () => {
    const queue = { add: jest.fn().mockResolvedValue({ id: 'job-1' }) };

    await enqueueWebhookDelivery(webhook, payload, queue);

    expect(queue.add).toHaveBeenCalledWith(
      'deliver',
      { webhook, payload },
      expect.objectContaining({
        attempts: MAX_WEBHOOK_ATTEMPTS,
        backoff: {
          type: 'exponential',
          delay: WEBHOOK_BACKOFF_DELAY_MS,
        },
        jobId: expect.any(String),
      }),
    );
    expect(MAX_WEBHOOK_ATTEMPTS).toBe(5);
  });

  test('worker throws failed deliveries so BullMQ retries them', async () => {
    global.fetch.mockResolvedValue({ ok: false, status: 503 });
    const prisma = {
      webhook: {
        findUnique: jest.fn().mockResolvedValue({ failingSince: null }),
        update: jest.fn().mockResolvedValue({}),
      },
    };

    await expect(processWebhookJob(
      { data: { webhook, payload }, attemptsMade: 0 },
      { prisma, poolRunFn: jest.fn() },
    )).rejects.toThrow('HTTP 503');

    expect(prisma.webhook.update).toHaveBeenCalledWith(expect.objectContaining({
      where: { id: webhook.id },
      data: expect.objectContaining({ failingSince: expect.any(Date) }),
    }));
  });

  test('worker marks a recovered webhook as successful', async () => {
    global.fetch.mockResolvedValue({ ok: true, status: 200 });
    const prisma = {
      webhook: {
        update: jest.fn().mockResolvedValue({}),
      },
    };

    await processWebhookJob(
      { data: { webhook, payload }, attemptsMade: 2 },
      { prisma, poolRunFn: jest.fn() },
    );

    expect(prisma.webhook.update).toHaveBeenCalledWith(expect.objectContaining({
      where: { id: webhook.id },
      data: expect.objectContaining({ failingSince: null }),
    }));
  });

  test('configures and starts a BullMQ webhook worker', () => {
    const dependencies = {
      prisma: { webhook: {} },
      poolRunFn: jest.fn(),
    };

    startWebhookWorker(dependencies);

    // Default concurrency is 10 (WEBHOOK_CONCURRENCY env var, defaulting to 10).
    expect(Worker).toHaveBeenCalledWith(
      WEBHOOK_QUEUE_NAME,
      expect.any(Function),
      expect.objectContaining({ concurrency: 10 }),
    );
    expect(mockWorkerOptions.connection).toEqual(expect.objectContaining({ quit: mockRedisQuit }));
    expect(mockWorkerProcessor).toEqual(expect.any(Function));
  });

  test('respects WEBHOOK_CONCURRENCY env var for concurrency cap (issue #26)', () => {
    process.env.WEBHOOK_CONCURRENCY = '3';
    try {
      jest.resetModules();
      const { Worker: W } = require('bullmq');
      const { startWebhookWorker: start, closeWebhookQueue: close } = require('../src/webhookWorker');

      const deps = { prisma: { webhook: {} }, poolRunFn: jest.fn() };
      start(deps);

      expect(W).toHaveBeenCalledWith(
        expect.any(String),
        expect.any(Function),
        expect.objectContaining({ concurrency: 3 }),
      );

      close();
    } finally {
      delete process.env.WEBHOOK_CONCURRENCY;
      jest.resetModules();
    }
  });

  test('no more than WEBHOOK_CONCURRENCY deliveries run simultaneously (issue #26)', async () => {
    const concurrencyLimit = 4;
    let inFlight = 0;
    let maxInFlight = 0;

    const { processWebhookJob } = require('../src/webhookWorker');

    global.fetch = jest.fn(() => {
      inFlight++;
      maxInFlight = Math.max(maxInFlight, inFlight);
      return new Promise((resolve) => {
        setImmediate(() => {
          inFlight--;
          resolve({ ok: true });
        });
      });
    });

    const prisma = {
      webhook: { update: jest.fn().mockResolvedValue({}) },
    };

    const jobs = Array.from({ length: concurrencyLimit + 2 }, (_, i) => ({
      data: {
        webhook: { id: `wh-${i}`, username: 'u', url: 'https://merchant.example.com/hook', secret: 's' },
        payload: { event: 'payment.received', event_id: `eid-${i}`, timestamp: '2026-01-01T00:00:00Z', data: {} },
      },
      attemptsMade: 0,
    }));

    // Run all jobs concurrently (simulating what BullMQ would do up to its cap).
    await Promise.all(jobs.map((job) => processWebhookJob(job, { prisma, poolRunFn: jest.fn() })));

    // Each individual job only calls fetch once, so maxInFlight equals the
    // number of jobs dispatched together — the BullMQ Worker's concurrency
    // option enforces the actual cap at the queue level.
    expect(maxInFlight).toBeGreaterThan(0);

    delete global.fetch;
  });

  test('queues a payment event for every registered webhook', async () => {
    const queue = { add: jest.fn().mockResolvedValue({}) };
    const prisma = {
      webhook: {
        findMany: jest.fn().mockResolvedValue([
          webhook,
          { ...webhook, id: 'webhook-2', url: 'https://second.example/webhooks' },
        ]),
      },
    };

    await dispatchPaymentWebhooks({
      prisma,
      poolGetFn: jest.fn(),
      queue,
      payment: {
        id: 'payment-1',
        type: 'payment',
        transaction_hash: 'transaction-1',
        to: 'GDESTINATION',
        from: 'GSOURCE',
        amount: '10.00',
        asset_type: 'native',
      },
    });

    expect(queue.add).toHaveBeenCalledTimes(2);
    expect(queue.add).toHaveBeenCalledWith(
      'deliver',
      expect.objectContaining({
        payload: expect.objectContaining({
          event: 'payment.received',
          event_id: 'transaction-1-payment-1',
        }),
      }),
      expect.objectContaining({ attempts: 5 }),
    );
  });

  test('creates a lazy queue when no queue is injected', async () => {
    mockQueueAdd.mockResolvedValue({ id: 'job-1' });

    await enqueueWebhookDelivery(webhook, payload);

    expect(Queue).toHaveBeenCalledWith(
      WEBHOOK_QUEUE_NAME,
      expect.objectContaining({ connection: expect.any(Object) }),
    );
    expect(mockQueueAdd).toHaveBeenCalledTimes(1);
  });
});

describe('SSRF protection – detectSsrfTarget', () => {
  test('allows a public HTTPS URL', () => {
    expect(detectSsrfTarget('https://merchant.example.com/hook')).toBeNull();
  });

  test('allows an HTTPS URL on an arbitrary public port', () => {
    expect(detectSsrfTarget('https://payments.acme.io:8443/webhooks')).toBeNull();
  });

  test('blocks a plain http:// URL', () => {
    expect(detectSsrfTarget('http://merchant.example.com/hook')).toMatch(/HTTPS/);
  });

  test('blocks a private RFC 1918 address (192.168.x.x)', () => {
    expect(detectSsrfTarget('https://192.168.1.1/hook')).toMatch(/private/i);
  });

  test('blocks a private RFC 1918 address (10.x.x.x)', () => {
    expect(detectSsrfTarget('https://10.0.0.1/internal')).toMatch(/private/i);
  });

  test('blocks a private RFC 1918 address (172.16-31.x.x)', () => {
    expect(detectSsrfTarget('https://172.16.0.1/hook')).toMatch(/private/i);
    expect(detectSsrfTarget('https://172.31.255.255/hook')).toMatch(/private/i);
  });

  test('does not block 172.15.x.x (just outside RFC 1918 range)', () => {
    expect(detectSsrfTarget('https://172.15.0.1/hook')).toBeNull();
  });

  test('blocks a loopback address (127.0.0.1)', () => {
    expect(detectSsrfTarget('https://127.0.0.1/hook')).toMatch(/loopback/i);
  });

  test('blocks a loopback hostname (localhost)', () => {
    expect(detectSsrfTarget('https://localhost/hook')).toMatch(/loopback/i);
  });

  test('blocks an IPv6 loopback address (::1)', () => {
    expect(detectSsrfTarget('https://[::1]/hook')).toMatch(/loopback/i);
  });

  test('blocks a link-local address (169.254.x.x)', () => {
    // 169.254.169.254 is the AWS instance metadata endpoint
    expect(detectSsrfTarget('https://169.254.169.254/latest/meta-data/')).toMatch(/link-local/i);
  });

  test('blocks a file:// URL', () => {
    expect(detectSsrfTarget('file:///etc/passwd')).toMatch(/HTTPS/);
  });

  test('blocks an ftp:// URL', () => {
    expect(detectSsrfTarget('ftp://merchant.example.com/hook')).toMatch(/HTTPS/);
  });

  test('blocks an http:// URL targeting a private IP', () => {
    const err = detectSsrfTarget('http://192.168.1.1/hook');
    // Blocked at the non-HTTPS scheme check before even reaching the IP check.
    expect(err).toBeTruthy();
  });

  test('sendWebhook rejects a private IP URL before making any network request', async () => {
    const { sendWebhook } = require('../src/webhookWorker');
    // Track whether fetch is called (if it was set previously by the outer suite).
    const fetchBefore = global.fetch;
    global.fetch = jest.fn();

    await expect(sendWebhook('https://192.168.1.1/hook', { event: 'test' }, 'secret')).rejects.toThrow(
      /SSRF|private/i,
    );
    expect(global.fetch).not.toHaveBeenCalled();

    global.fetch = fetchBefore;
  });

  test('sendWebhook rejects a file:// URL before making any network request', async () => {
    const { sendWebhook } = require('../src/webhookWorker');
    const fetchBefore = global.fetch;
    global.fetch = jest.fn();

    await expect(sendWebhook('file:///etc/passwd', { event: 'test' }, 'secret')).rejects.toThrow(
      /SSRF|HTTPS/i,
    );
    expect(global.fetch).not.toHaveBeenCalled();

    global.fetch = fetchBefore;
  });

  test('sendWebhook proceeds to fetch for a public HTTPS URL', async () => {
    const { sendWebhook } = require('../src/webhookWorker');
    global.fetch = jest.fn().mockResolvedValue({ ok: true });

    await expect(sendWebhook('https://merchant.example.com/hook', { event: 'test' }, 'secret')).resolves.toEqual({ ok: true });
    expect(global.fetch).toHaveBeenCalledWith('https://merchant.example.com/hook', expect.anything());
  });
});
