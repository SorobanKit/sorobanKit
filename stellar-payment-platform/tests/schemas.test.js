'use strict';

const {
  registerBodySchema,
  federationQuerySchema,
  lookupQuerySchema,
  usersQuerySchema,
  accountPaymentsQuerySchema,
  verifyEmailBodySchema,
  verifyEmailConfirmBodySchema,
  adminBlockBodySchema,
  adminRoutingStatsQuerySchema,
  webhookBodySchema,
  webhookQuerySchema,
} = require('../src/schemas');

const messagesFor = (schema, input) => {
  const result = schema.safeParse(input);
  if (result.success) return null;
  return result.error.issues.map((issue) => `${issue.path.join('.')}: ${issue.message}`);
};

describe('request schemas', () => {
  describe('registerBodySchema', () => {
    const valid = { username: 'alice123', address: 'GABC123XYZ456789' };

    test('accepts a bare alphanumeric username', () => {
      expect(registerBodySchema.safeParse(valid).success).toBe(true);
    });

    test.each([3, 20])('accepts a %i character username', (length) => {
      expect(registerBodySchema.safeParse({ ...valid, username: 'a'.repeat(length) }).success).toBe(true);
    });

    test.each([
      ['missing username', { username: undefined }, 'username: username is required'],
      ['2 character username', { username: 'ab' }, 'username: username must be between 3 and 20 characters'],
      ['21 character username', { username: 'a'.repeat(21) }, 'username: username must be between 3 and 20 characters'],
      ['punctuation', { username: 'alice!@#' }, 'username: username must contain only letters and numbers'],
      ['a federation tag', { username: 'ab*domain.com' }, 'username: username must contain only letters and numbers'],
      ['a non-string username', { username: 42 }, 'username: username is required'],
    ])('rejects %s', (_label, patch, expected) => {
      const messages = messagesFor(registerBodySchema, { ...valid, ...patch });
      expect(messages).toContain(expected);
    });

    test('requires a non-empty address', () => {
      expect(messagesFor(registerBodySchema, { ...valid, address: '' })).toContain(
        'address: address cannot be empty',
      );
      expect(messagesFor(registerBodySchema, { username: 'alice123' })).toContain(
        'address: address is required',
      );
    });

    test('does not bound address length, so StrKey stays the format authority', () => {
      const long = "' UNION SELECT username, address, created_at FROM username_registry --";
      expect(registerBodySchema.safeParse({ ...valid, address: long }).success).toBe(true);
    });

    test('leaves memo rules to validateMemo', () => {
      expect(registerBodySchema.safeParse({ ...valid, memo_type: 'bogus', memo: 'x' }).success).toBe(true);
      expect(registerBodySchema.safeParse({ ...valid, memo: 'orphan' }).success).toBe(true);
    });

    test('trims surrounding whitespace', () => {
      const parsed = registerBodySchema.parse({ username: '  alice123  ', address: '  GABC  ' });
      expect(parsed).toMatchObject({ username: 'alice123', address: 'GABC' });
    });

    test('normalises username to lowercase', () => {
      const parsed = registerBodySchema.parse({ username: 'Alice123', address: 'GABC' });
      expect(parsed.username).toBe('alice123');
    });

    test('normalises mixed-case username to lowercase', () => {
      const parsed = registerBodySchema.parse({ username: 'ALICE', address: 'GABC' });
      expect(parsed.username).toBe('alice');
    });

    test('Alice and alice map to the same slot after normalisation', () => {
      const upper = registerBodySchema.parse({ username: 'Alice', address: 'GABC' });
      const lower = registerBodySchema.parse({ username: 'alice', address: 'GABC' });
      expect(upper.username).toBe(lower.username);
    });

    test('keeps unrecognised fields rather than silently dropping them', () => {
      const parsed = registerBodySchema.parse({ ...valid, 'Idempotency-Key': 'abc' });
      expect(parsed['Idempotency-Key']).toBe('abc');
    });
  });

  describe('federationQuerySchema', () => {
    test('requires q', () => {
      expect(messagesFor(federationQuerySchema, {})).toContain("q: Missing 'q' parameter");
      expect(messagesFor(federationQuerySchema, { q: '   ' })).toContain("q: Missing 'q' parameter");
    });

    test.each(['id', 'name'])('accepts type=%s', (type) => {
      expect(federationQuerySchema.safeParse({ q: 'alice*localhost', type }).success).toBe(true);
    });

    test('rejects an unsupported type', () => {
      expect(messagesFor(federationQuerySchema, { q: 'alice*localhost', type: 'bogus' })).toContain(
        "type: Unsupported query type. Supported types: 'id', 'name'",
      );
    });

    test('treats an absent type as valid', () => {
      expect(federationQuerySchema.safeParse({ q: 'alice*localhost' }).success).toBe(true);
    });

    test('accepts a well-formed federation address', () => {
      expect(federationQuerySchema.safeParse({ q: 'alice*localhost' }).success).toBe(true);
      expect(federationQuerySchema.safeParse({ q: 'alice*localhost', type: 'name' }).success).toBe(true);
    });

    test.each([
      ['missing q', {}],
      ['whitespace-only q', { q: '   ' }],
      ['q without a separator', { q: 'nodomain' }],
      ['q with two separators', { q: 'alice*localhost*extra' }],
      ['q with an empty username part', { q: '*domain' }],
      ['q with an empty domain part', { q: 'alice*' }],
      ['q longer than 255 characters', { q: `${'a'.repeat(300)}*domain` }],
    ])('rejects %s', (_label, input) => {
      expect(federationQuerySchema.safeParse(input).success).toBe(false);
    });
  });

  describe('lookupQuerySchema', () => {
    test('requires address or search', () => {
      expect(messagesFor(lookupQuerySchema, {})).toContain(
        "address: Missing required parameter: provide 'address' for exact lookup or 'search' for paginated search",
      );
    });

    test.each([{ address: 'GABC' }, { search: 'ali' }])('accepts %o', (input) => {
      expect(lookupQuerySchema.safeParse(input).success).toBe(true);
    });

    test('defaults pagination', () => {
      expect(lookupQuerySchema.parse({ address: 'GABC' })).toMatchObject({ page: 1, limit: 10 });
    });

    test('passes injection-style payloads through untouched', () => {
      const payload = "' OR 1=1 --";
      expect(lookupQuerySchema.parse({ address: payload }).address).toBe(payload);
    });
  });

  describe('pagination coercion', () => {
    test('parses numeric strings', () => {
      expect(usersQuerySchema.parse({ page: '3', limit: '5' })).toMatchObject({ page: 3, limit: 5 });
    });

    // ── Acceptance-criteria boundary cases ────────────────────────────────────

    test('?page=-1 is clamped to page=1', () => {
      expect(usersQuerySchema.parse({ page: '-1' })).toMatchObject({ page: 1 });
    });

    test('?limit=0 is clamped to limit=1', () => {
      expect(usersQuerySchema.parse({ limit: '0' })).toMatchObject({ limit: 1 });
    });

    test('?limit=1000 is clamped to limit=100', () => {
      expect(usersQuerySchema.parse({ limit: '1000' })).toMatchObject({ limit: 100 });
    });

    test('missing page defaults to 1', () => {
      expect(usersQuerySchema.parse({})).toMatchObject({ page: 1 });
    });

    test('missing limit defaults to 10', () => {
      expect(usersQuerySchema.parse({})).toMatchObject({ limit: 10 });
    });

    // ── Additional edge cases ─────────────────────────────────────────────────

    test.each([
      ['clamps limit above the maximum', { limit: '1000' }, { limit: 100 }],
      ['clamps limit below the minimum', { limit: '-5' }, { limit: 1 }],
      ['clamps limit at exactly 0', { limit: '0' }, { limit: 1 }],
      ['clamps limit at exactly 1 (boundary, valid)', { limit: '1' }, { limit: 1 }],
      ['clamps limit at exactly 100 (boundary, valid)', { limit: '100' }, { limit: 100 }],
      ['clamps limit at exactly 101', { limit: '101' }, { limit: 100 }],
      ['falls back on non-numeric limit', { limit: 'abc' }, { limit: 10 }],
      ['falls back on non-numeric page', { page: 'abc' }, { page: 1 }],
      ['clamps page below the minimum', { page: '0' }, { page: 1 }],
      ['clamps page at -1', { page: '-1' }, { page: 1 }],
      ['accepts page=1 (boundary, valid)', { page: '1' }, { page: 1 }],
      ['accepts page=2', { page: '2' }, { page: 2 }],
      ['falls back on empty string limit', { limit: '' }, { limit: 10 }],
      ['falls back on empty string page', { page: '' }, { page: 1 }],
    ])('%s', (_label, input, expected) => {
      expect(usersQuerySchema.parse(input)).toMatchObject(expected);
    });

    test('applies the payments endpoint default of 25', () => {
      expect(accountPaymentsQuerySchema.parse({})).toMatchObject({ limit: 25, order: 'desc' });
    });

    test('payments endpoint clamps limit=0 to 1', () => {
      expect(accountPaymentsQuerySchema.parse({ limit: '0' })).toMatchObject({ limit: 1 });
    });

    test('payments endpoint clamps limit=1000 to 100', () => {
      expect(accountPaymentsQuerySchema.parse({ limit: '1000' })).toMatchObject({ limit: 100 });
    });

    test('falls back to desc for an unknown order', () => {
      expect(accountPaymentsQuerySchema.parse({ order: 'sideways' }).order).toBe('desc');
      expect(accountPaymentsQuerySchema.parse({ order: 'asc' }).order).toBe('asc');
    });
  });

  describe('webhookBodySchema', () => {
    const valid = { url: 'https://example.com/webhooks/stellar' };

    test('accepts a valid HTTPS URL', () => {
      expect(webhookBodySchema.safeParse(valid).success).toBe(true);
    });

    test('rejects a missing url field', () => {
      const messages = messagesFor(webhookBodySchema, {});
      expect(messages).not.toBeNull();
      expect(messages.some((message) => message.startsWith('url:'))).toBe(true);
    });

    test('rejects a non-HTTPS URL', () => {
      const messages = messagesFor(webhookBodySchema, { url: 'http://example.com/webhooks' });
      expect(messages).not.toBeNull();
      expect(messages.some((message) => message.startsWith('url:'))).toBe(true);
    });

    test('rejects a URL longer than the maximum length', () => {
      const longUrl = `https://example.com/${'a'.repeat(2048)}`;
      const messages = messagesFor(webhookBodySchema, { url: longUrl });
      expect(messages).not.toBeNull();
      expect(messages.some((message) => message.startsWith('url:'))).toBe(true);
    });

    test('rejects an invalid event type', () => {
      const messages = messagesFor(webhookBodySchema, { ...valid, events: ['not_a_real_event'] });
      expect(messages).not.toBeNull();
      expect(messages.some((message) => message.startsWith('events'))).toBe(true);
    });

    test('accepts a valid event type', () => {
      expect(webhookBodySchema.safeParse({ ...valid, events: ['payment.received'] }).success).toBe(true);
    });
  });

  describe('webhookQuerySchema', () => {
    test('coerces numeric string pagination parameters', () => {
      expect(webhookQuerySchema.parse({ page: '3', limit: '5' })).toMatchObject({ page: 3, limit: 5 });
    });

    test('defaults a missing page to 1', () => {
      expect(webhookQuerySchema.parse({})).toMatchObject({ page: 1 });
    });

    test('defaults a missing limit to the documented default', () => {
      const parsed = webhookQuerySchema.parse({});
      expect(typeof parsed.limit).toBe('number');
      expect(parsed.limit).toBeGreaterThanOrEqual(1);
      expect(parsed.limit).toBeLessThanOrEqual(100);
    });

    test.each([
      ['clamps limit above the maximum', { limit: '1000' }, { limit: 100 }],
      ['clamps limit below the minimum', { limit: '-5' }, { limit: 1 }],
      ['clamps limit at exactly 0', { limit: '0' }, { limit: 1 }],
      ['clamps page below the minimum', { page: '0' }, { page: 1 }],
      ['clamps page at -1', { page: '-1' }, { page: 1 }],
    ])('%s', (_label, input, expected) => {
      expect(webhookQuerySchema.parse(input)).toMatchObject(expected);
    });
  });

  describe('auth and admin schemas', () => {
    test('requires a well-formed email', () => {
      expect(verifyEmailBodySchema.safeParse({ email: 'a@b.com' }).success).toBe(true);
      expect(messagesFor(verifyEmailBodySchema, { email: 'nope' })).toContain(
        'email: a valid email is required',
      );
      expect(messagesFor(verifyEmailBodySchema, {})).toContain('email: email is required');
    });

    test('requires a 6-digit confirmation code', () => {
      expect(verifyEmailConfirmBodySchema.safeParse({ email: 'a@b.com', code: '123456' }).success).toBe(true);
      expect(messagesFor(verifyEmailConfirmBodySchema, { email: 'a@b.com', code: '12345' })).toContain(
        'code: code must be a 6-digit number',
      );
      expect(messagesFor(verifyEmailConfirmBodySchema, { email: 'a@b.com', code: 'abcdef' })).toContain(
        'code: code must be a 6-digit number',
      );
    });

    test('requires an address to block', () => {
      expect(messagesFor(adminBlockBodySchema, {})).toContain('address: Missing or invalid address');
      expect(messagesFor(adminBlockBodySchema, { address: 42 })).toContain(
        'address: Missing or invalid address',
      );
      expect(adminBlockBodySchema.safeParse({ address: 'GABC' }).success).toBe(true);
    });

    test('validates adminRoutingStatsQuerySchema', () => {
      expect(adminRoutingStatsQuerySchema.safeParse({}).success).toBe(true);
      expect(adminRoutingStatsQuerySchema.parse({})).toMatchObject({ groupBy: 'day' });
      expect(
        adminRoutingStatsQuerySchema.safeParse({
          startDate: '2026-01-01',
          endDate: '2026-01-31',
          groupBy: 'week',
          assetCode: 'XLM',
        }).success,
      ).toBe(true);

      expect(
        messagesFor(adminRoutingStatsQuerySchema, { startDate: 'bad' }),
      ).toContain('startDate: startDate must be YYYY-MM-DD');

      expect(
        messagesFor(adminRoutingStatsQuerySchema, {
          startDate: '2026-02-01',
          endDate: '2026-01-01',
        }),
      ).toContain('startDate: startDate must be on or before endDate');
    });
  });
});

