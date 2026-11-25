'use strict';

const { CircuitBreaker, CircuitBreakerOpenError, STATES } = require('./circuitBreaker');

function makeBreaker(opts = {}) {
  return new CircuitBreaker('test', { failureThreshold: 3, recoveryTimeoutMs: 50, successThreshold: 2, ...opts });
}

describe('CircuitBreaker', () => {
  it('starts CLOSED and passes through successes', async () => {
    const cb = makeBreaker();
    const result = await cb.execute(() => Promise.resolve('ok'));
    expect(result).toBe('ok');
    expect(cb.state).toBe(STATES.CLOSED);
  });

  it('opens after failureThreshold consecutive failures', async () => {
    const cb = makeBreaker({ failureThreshold: 2 });
    for (let i = 0; i < 2; i++) {
      await expect(cb.execute(() => Promise.reject(new Error('fail')))).rejects.toThrow('fail');
    }
    expect(cb.state).toBe(STATES.OPEN);
  });

  it('throws CircuitBreakerOpenError when open', async () => {
    const cb = makeBreaker({ failureThreshold: 1 });
    await expect(cb.execute(() => Promise.reject(new Error('x')))).rejects.toThrow();
    await expect(cb.execute(() => Promise.resolve('ok'))).rejects.toBeInstanceOf(CircuitBreakerOpenError);
  });

  it('transitions to HALF_OPEN after recovery timeout', async () => {
    const cb = makeBreaker({ failureThreshold: 1, recoveryTimeoutMs: 20 });
    await expect(cb.execute(() => Promise.reject(new Error('x')))).rejects.toThrow();
    await new Promise((r) => setTimeout(r, 30));
    // Trigger state check by calling execute
    cb._maybeTransitionFromOpen();
    expect(cb.state).toBe(STATES.HALF_OPEN);
  });

  it('closes after successThreshold successes in HALF_OPEN', async () => {
    const cb = makeBreaker({ failureThreshold: 1, recoveryTimeoutMs: 10, successThreshold: 2 });
    await expect(cb.execute(() => Promise.reject(new Error('x')))).rejects.toThrow();
    await new Promise((r) => setTimeout(r, 20));
    cb._maybeTransitionFromOpen();
    await cb.execute(() => Promise.resolve('p1'));
    await cb.execute(() => Promise.resolve('p2'));
    expect(cb.state).toBe(STATES.CLOSED);
  });
});
