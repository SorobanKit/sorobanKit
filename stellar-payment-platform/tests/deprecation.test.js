'use strict';

const deprecation = require('../src/middleware/deprecation');
const { DEPRECATIONS } = require('../src/config/deprecations');

const makeRes = () => {
  const headers = {};
  return {
    headers,
    set(key, value) {
      headers[key] = value;
      return this;
    },
    setHeader(key, value) {
      headers[key] = value;
      return this;
    },
    getHeader(key) {
      return headers[key];
    },
  };
};

describe('Deprecation middleware - Sunset header RFC 8594', () => {
  it('sets a Sunset header in HTTP date format for deprecated routes', () => {
    const mw = deprecation.deprecationMiddleware();
    const req = { method: 'GET', path: '/api/v1/lookup' };
    const res = makeRes();
    let nextCalled = false;

    mw(req, res, () => {
      nextCalled = true;
    });

    expect(nextCalled).toBe(true);
    expect(res.headers.Deprecation).toBeDefined();
    expect(res.headers.Sunset).toBeDefined();

    const parsedDate = new Date(res.headers.Sunset);
    expect(Number.isNaN(parsedDate.getTime())).toBe(false);
    expect(res.headers.Sunset).toMatch(/^[A-Z][a-z]{2}, \d{2} [A-Z][a-z]{2} \d{4} \d{2}:\d{2}:\d{2} GMT$/);
  });

  it('ensures Sunset date matches the value in deprecations.js', () => {
    const mw = deprecation.deprecationMiddleware();

    for (const entry of DEPRECATIONS) {
      const req = { method: entry.method, path: entry.path };
      const res = makeRes();

      mw(req, res, () => {});

      const expectedDate = new Date(entry.sunsetDate).toUTCString();
      expect(res.headers.Sunset).toBe(expectedDate);
    }
  });

  it('does not include a Sunset header for non-deprecated routes', () => {
    const mw = deprecation.deprecationMiddleware();
    const req = { method: 'GET', path: '/api/v2/active-endpoint' };
    const res = makeRes();
    let nextCalled = false;

    mw(req, res, () => {
      nextCalled = true;
    });

    expect(nextCalled).toBe(true);
    expect(res.headers.Sunset).toBeUndefined();
    expect(res.headers.Deprecation).toBeUndefined();
  });
});
