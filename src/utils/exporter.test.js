'use strict';

const {
  csvEscape,
  memoToCsvRow,
  exportToCsv,
  exportToNdjson,
} = require('./exporter');

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

describe('src/utils/exporter.js', () => {
  it('escapes fields containing commas with enclosing double quotes', () => {
    expect(csvEscape('groceries, fruit, and snacks')).toBe('"groceries, fruit, and snacks"');
  });

  it('escapes fields containing double quotes by doubling internal quotes and enclosing in quotes', () => {
    expect(csvEscape('invoice "INV-99" approved')).toBe('"invoice ""INV-99"" approved"');
  });

  it('escapes fields containing newlines with enclosing double quotes', () => {
    expect(csvEscape('First line\nSecond line')).toBe('"First line\nSecond line"');
  });

  it('handles combination of commas, double quotes, and newlines', () => {
    expect(csvEscape('Header,\n"Item 1", "Item 2"')).toBe('"Header,\n""Item 1"", ""Item 2"""');
  });

  it('parses exported CSV accurately with standard CSV parser', () => {
    const memos = [
      { id: '1', content: 'Normal' },
      { id: '2', content: 'With, comma' },
      { id: '3', content: 'With "quotes"' },
      { id: '4', content: 'With\nnewline' },
    ];
    const csv = exportToCsv(memos);
    const parsed = parseCsv(csv);
    expect(parsed[1][1]).toBe('Normal');
    expect(parsed[2][1]).toBe('With, comma');
    expect(parsed[3][1]).toBe('With "quotes"');
    expect(parsed[4][1]).toBe('With\nnewline');
  });
});
