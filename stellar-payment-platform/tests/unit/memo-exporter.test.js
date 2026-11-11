'use strict';

const {
  csvEscape,
  memoToCsvRow,
  exportToCsv,
  exportToNdjson,
} = require('../../../src/utils/exporter');

/**
 * Standard RFC 4180 CSV parser to validate the output against.
 */
function parseCsv(csvString) {
  const rows = [];
  let currentRow = [];
  let currentField = '';
  let insideQuotes = false;
  let i = 0;

  while (i < csvString.length) {
    const char = csvString[i];
    const nextChar = csvString[i + 1];

    if (insideQuotes) {
      if (char === '"') {
        if (nextChar === '"') {
          currentField += '"';
          i += 2;
          continue;
        } else {
          insideQuotes = false;
          i++;
          continue;
        }
      } else {
        currentField += char;
        i++;
      }
    } else {
      if (char === '"') {
        insideQuotes = true;
        i++;
      } else if (char === ',') {
        currentRow.push(currentField);
        currentField = '';
        i++;
      } else if (char === '\n' || char === '\r') {
        if (char === '\r' && nextChar === '\n') {
          i++;
        }
        currentRow.push(currentField);
        rows.push(currentRow);
        currentRow = [];
        currentField = '';
        i++;
      } else {
        currentField += char;
        i++;
      }
    }
  }

  if (currentField || currentRow.length > 0) {
    currentRow.push(currentField);
    rows.push(currentRow);
  }

  return rows;
}

describe('src/utils/exporter.js - csvEscape & RFC 4180 memo escaping', () => {
  describe('csvEscape', () => {
    it('returns empty string for null and undefined', () => {
      expect(csvEscape(null)).toBe('');
      expect(csvEscape(undefined)).toBe('');
    });

    it('returns unquoted string for regular alphanumeric values', () => {
      expect(csvEscape('memo123')).toBe('memo123');
      expect(csvEscape(456)).toBe('456');
    });

    it('escapes fields containing commas with enclosing double quotes', () => {
      const res = csvEscape('groceries, fruit, and snacks');
      expect(res).toBe('"groceries, fruit, and snacks"');
    });

    it('escapes fields containing double quotes by doubling internal quotes and enclosing in quotes', () => {
      const res = csvEscape('invoice "INV-99" approved');
      expect(res).toBe('"invoice ""INV-99"" approved"');
    });

    it('escapes fields containing newlines with enclosing double quotes', () => {
      const res = csvEscape('First line\nSecond line');
      expect(res).toBe('"First line\nSecond line"');
    });

    it('escapes fields containing CRLF line breaks', () => {
      const res = csvEscape('Line1\r\nLine2');
      expect(res).toBe('"Line1\r\nLine2"');
    });

    it('handles combination of commas, double quotes, and newlines', () => {
      const res = csvEscape('Header,\n"Item 1", "Item 2"');
      expect(res).toBe('"Header,\n""Item 1"", ""Item 2"""');
    });
  });

  describe('memoToCsvRow and exportToCsv', () => {
    it('formats a memo with a comma into a valid CSV field', () => {
      const memo = {
        id: 'memo-1',
        content: 'Transfer to Alice, Bob',
        createdAt: '2026-01-01T00:00:00Z',
        updatedAt: '2026-01-01T00:00:00Z',
        metadata: 'tag1',
      };
      const row = memoToCsvRow(memo);
      expect(row).toContain('"Transfer to Alice, Bob"');

      const parsed = parseCsv(row);
      expect(parsed[0][1]).toBe('Transfer to Alice, Bob');
    });

    it('formats a memo with double quotes into a valid CSV field', () => {
      const memo = {
        id: 'memo-2',
        content: 'Memo with "special quotation"',
        createdAt: '2026-01-01T00:00:00Z',
        updatedAt: '2026-01-01T00:00:00Z',
        metadata: null,
      };
      const row = memoToCsvRow(memo);
      expect(row).toContain('"Memo with ""special quotation"""');

      const parsed = parseCsv(row);
      expect(parsed[0][1]).toBe('Memo with "special quotation"');
    });

    it('formats a memo with newlines into a valid CSV field', () => {
      const memo = {
        id: 'memo-3',
        content: 'Line 1\nLine 2\nLine 3',
        createdAt: '2026-01-01T00:00:00Z',
        updatedAt: '2026-01-01T00:00:00Z',
        metadata: null,
      };
      const row = memoToCsvRow(memo);
      expect(row).toContain('"Line 1\nLine 2\nLine 3"');

      const parsed = parseCsv(row);
      expect(parsed[0][1]).toBe('Line 1\nLine 2\nLine 3');
    });

    it('exports a full collection of memos that parses accurately with a standard CSV parser', () => {
      const memos = [
        {
          id: '1',
          content: 'Simple content',
          createdAt: '2026-01-01T00:00:00Z',
          updatedAt: '2026-01-01T00:00:00Z',
          metadata: 'none',
        },
        {
          id: '2',
          content: 'Comma, in, content',
          createdAt: '2026-01-02T00:00:00Z',
          updatedAt: '2026-01-02T00:00:00Z',
          metadata: 'test',
        },
        {
          id: '3',
          content: 'Quote "here" and "there"',
          createdAt: '2026-01-03T00:00:00Z',
          updatedAt: '2026-01-03T00:00:00Z',
          metadata: { category: 'urgent' },
        },
        {
          id: '4',
          content: 'Multi\nLine\nMemo',
          createdAt: '2026-01-04T00:00:00Z',
          updatedAt: '2026-01-04T00:00:00Z',
          metadata: null,
        },
      ];

      const csv = exportToCsv(memos);
      const parsed = parseCsv(csv);

      // Header row
      expect(parsed[0]).toEqual(['id', 'content', 'createdAt', 'updatedAt', 'metadata']);

      // Data rows
      expect(parsed[1][1]).toBe('Simple content');
      expect(parsed[2][1]).toBe('Comma, in, content');
      expect(parsed[3][1]).toBe('Quote "here" and "there"');
      expect(parsed[4][1]).toBe('Multi\nLine\nMemo');
    });
  });

  describe('exportToNdjson', () => {
    it('exports memos as newline-delimited JSON objects', () => {
      const memos = [
        { id: '1', content: 'hello' },
        { id: '2', content: 'world' },
      ];
      const ndjson = exportToNdjson(memos);
      const lines = ndjson.split('\n');
      expect(JSON.parse(lines[0])).toEqual(memos[0]);
      expect(JSON.parse(lines[1])).toEqual(memos[1]);
    });
  });
});
