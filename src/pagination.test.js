const { paginate, clamp } = require('./pagination');

describe('paginate', () => {
  it('computes totalPages from total and limit', () => {
    const result = paginate({ total: 42, limit: 10, page: 1 });
    expect(result.total).toBe(42);
    expect(result.totalPages).toBe(5);
  });

  it('clamps negative totals to zero', () => {
    const warn = jest.spyOn(console, 'warn').mockImplementation(() => {});
    const result = paginate({ total: -5, limit: 10, page: 1 });
    expect(result.total).toBe(0);
    expect(result.totalPages).toBe(0);
    expect(warn).toHaveBeenCalled();
    warn.mockRestore();
  });

  it('handles a zero total', () => {
    const result = paginate({ total: 0, limit: 10, page: 1 });
    expect(result.total).toBe(0);
    expect(result.totalPages).toBe(0);
  });

  it('falls back to defaults when options are missing or invalid', () => {
    const emptyResult = paginate();
    expect(emptyResult).toEqual({ page: 1, pageSize: 10, total: 0, totalPages: 0 });

    const invalidResult = paginate({ page: -2, pageSize: 'invalid', total: 'invalid' });
    expect(invalidResult).toEqual({ page: 1, pageSize: 10, total: 0, totalPages: 0 });
  });

});

describe('clamp', () => {
  it('returns value when within min and max range', () => {
    expect(clamp(5, 1, 10)).toBe(5);
    expect(clamp(1, 1, 10)).toBe(1);
    expect(clamp(10, 1, 10)).toBe(10);
    expect(clamp(50, 1, 100)).toBe(50);
  });

  it('clamps values below minimum to min', () => {
    expect(clamp(-5, 1, 10)).toBe(1);
    expect(clamp(-100, 0, 50)).toBe(0);
    expect(clamp(0, 1, 10)).toBe(1);
  });

  it('clamps values above maximum to max', () => {
    expect(clamp(15, 1, 10)).toBe(10);
    expect(clamp(999, 1, 100)).toBe(100);
    expect(clamp(101, 1, 100)).toBe(100);
  });

  it('coerces valid numeric strings to numbers and clamps', () => {
    expect(clamp('5', 1, 10)).toBe(5);
    expect(clamp('0', 1, 10)).toBe(1);
    expect(clamp('-10', 1, 10)).toBe(1);
    expect(clamp('25', 1, 10)).toBe(10);
    expect(clamp('100', 1, 100)).toBe(100);
  });

  it('handles zero correctly across different bounds', () => {
    expect(clamp(0, 1, 10)).toBe(1);
    expect(clamp(0, -10, 10)).toBe(0);
    expect(clamp(0, 0, 10)).toBe(0);
    expect(clamp(0, -5, 0)).toBe(0);
    expect(clamp(0, -10, -1)).toBe(-1);
  });

  it('falls back to min for non-numeric and non-finite inputs', () => {
    expect(clamp('invalid', 1, 10)).toBe(1);
    expect(clamp('', 1, 10)).toBe(1);
    expect(clamp('   ', 1, 10)).toBe(1);
    expect(clamp(null, 1, 10)).toBe(1);
    expect(clamp(undefined, 1, 10)).toBe(1);
    expect(clamp(NaN, 1, 10)).toBe(1);
    expect(clamp(Infinity, 1, 10)).toBe(1);
    expect(clamp(-Infinity, 1, 10)).toBe(1);
  });

  it('uses default bounds (min=1, max=100) when omitted', () => {
    expect(clamp(50)).toBe(50);
    expect(clamp(-10)).toBe(1);
    expect(clamp(250)).toBe(100);
  });
});

