'use strict';

const { Bulkhead, BulkheadRejectedError } = require('./bulkhead');

function makeBulkhead(opts = {}) {
  return new Bulkhead('test', { concurrency: 2, queueDepth: 2, queueTimeoutMs: 50, ...opts });
}

describe('Bulkhead', () => {
  it('runs calls up to concurrency without queueing', async () => {
    const bh = makeBulkhead({ concurrency: 3 });
    const results = await Promise.all([
      bh.execute(() => Promise.resolve(1)),
      bh.execute(() => Promise.resolve(2)),
      bh.execute(() => Promise.resolve(3)),
    ]);
    expect(results).toEqual([1, 2, 3]);
  });

  it('queues calls beyond concurrency and runs them after slots free', async () => {
    const bh = makeBulkhead({ concurrency: 1, queueDepth: 2 });
    let resolveFirst;
    const first = new Promise((r) => { resolveFirst = r; });
    const p1 = bh.execute(() => first);
    const p2 = bh.execute(() => Promise.resolve('queued'));
    expect(bh.queueLength).toBe(1);
    resolveFirst('first');
    await expect(p1).resolves.toBe('first');
    await expect(p2).resolves.toBe('queued');
  });

  it('rejects with BulkheadRejectedError when queue is full', async () => {
    const bh = makeBulkhead({ concurrency: 1, queueDepth: 1 });
    let r1, r2;
    const h1 = new Promise((r) => { r1 = r; });
    const h2 = new Promise((r) => { r2 = r; });
    bh.execute(() => h1);         // fills concurrency slot
    bh.execute(() => h2);         // fills queue
    await expect(bh.execute(() => Promise.resolve())).rejects.toBeInstanceOf(BulkheadRejectedError);
    r1(); r2();
  });

  it('rejects queued calls that wait beyond queueTimeoutMs', async () => {
    const bh = makeBulkhead({ concurrency: 1, queueDepth: 5, queueTimeoutMs: 20 });
    let releaseSlot;
    const occupant = new Promise((r) => { releaseSlot = r; });
    bh.execute(() => occupant);   // occupies slot
    const queued = bh.execute(() => Promise.resolve('late'));
    await expect(queued).rejects.toBeInstanceOf(BulkheadRejectedError);
    releaseSlot();
  });
});
