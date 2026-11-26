'use strict';

const { withDeadline, DeadlineError } = require('./deadline');

describe('withDeadline', () => {
  it('resolves normally when signal is null', async () => {
    await expect(withDeadline(Promise.resolve(42), null)).resolves.toBe(42);
  });

  it('rejects with DeadlineError when signal is already aborted', async () => {
    const ctrl = new AbortController();
    ctrl.abort();
    await expect(withDeadline(Promise.resolve(42), ctrl.signal)).rejects.toBeInstanceOf(DeadlineError);
  });

  it('rejects with DeadlineError when signal fires before promise resolves', async () => {
    const ctrl = new AbortController();
    const slow = new Promise((resolve) => setTimeout(resolve, 500, 'late'));
    setTimeout(() => ctrl.abort(), 10);
    await expect(withDeadline(slow, ctrl.signal)).rejects.toBeInstanceOf(DeadlineError);
  });

  it('resolves if promise beats the signal', async () => {
    const ctrl = new AbortController();
    const fast = Promise.resolve('fast');
    setTimeout(() => ctrl.abort(), 100);
    await expect(withDeadline(fast, ctrl.signal)).resolves.toBe('fast');
  });

  it('propagates non-deadline rejections', async () => {
    const ctrl = new AbortController();
    const boom = Promise.reject(new Error('oops'));
    await expect(withDeadline(boom, ctrl.signal)).rejects.toThrow('oops');
  });
});
