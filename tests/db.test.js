'use strict';

describe('src/db.js DATABASE_URL check', () => {
  const originalEnv = process.env.DATABASE_URL;

  afterEach(() => {
    if (originalEnv !== undefined) {
      process.env.DATABASE_URL = originalEnv;
    } else {
      delete process.env.DATABASE_URL;
    }
  });

  it('logs an error and throws a clear message when DATABASE_URL is not set', () => {
    delete process.env.DATABASE_URL;
    const errorSpy = jest.spyOn(console, 'error').mockImplementation(() => {});

    expect(() => {
      delete require.cache[require.resolve('../src/db')];
      require('../src/db');
    }).toThrow('DATABASE_URL is not set — see README for database setup');

    expect(errorSpy).toHaveBeenCalledWith('DATABASE_URL is not set — see README for database setup');
    errorSpy.mockRestore();
  });
});
