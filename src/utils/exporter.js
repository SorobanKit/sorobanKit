/**
 * Export utilities for memos.
 *
 * Provides CSV and NDJSON serialization of memo collections.
 */

/**
 * Escape a single CSV field per RFC 4180.
 *
 * Wraps the value in double quotes and escapes any internal double-quote
 * characters by doubling them. This keeps fields containing commas,
 * newlines, or quotes from breaking the row structure.
 *
 * @param {*} field - Value to escape. `null`/`undefined` become empty strings.
 * @returns {string} The quoted, escaped field.
 */
function csvEscape(field) {
  const value = field === null || field === undefined ? '' : String(field);
  return '"' + value.replace(/"/g, '""') + '"';
}

/**
 * Serialize a single memo into a CSV row.
 *
 * @param {Object} memo - The memo to serialize.
 * @returns {string} A CSV row (without trailing newline).
 */
function memoToCsvRow(memo) {
  const fields = [
    memo.id,
    memo.content,
    memo.createdAt,
    memo.updatedAt,
    memo.metadata,
  ];
  return fields.map(csvEscape).join(',');
}

/**
 * Export a collection of memos as a CSV document.
 *
 * @param {Array<Object>} memos - The memos to export.
 * @returns {string} The CSV document.
 */
function exportToCsv(memos) {
  const header = ['id', 'content', 'createdAt', 'updatedAt', 'metadata'];
  const lines = [header.map(csvEscape).join(',')];
  for (const memo of memos) {
    lines.push(memoToCsvRow(memo));
  }
  return lines.join('\n');
}

/**
 * Export a collection of memos as NDJSON.
 *
 * @param {Array<Object>} memos - The memos to export.
 * @returns {string} The NDJSON document.
 */
function exportToNdjson(memos) {
  return memos.map((memo) => JSON.stringify(memo)).join('\n');
}

module.exports = {
  csvEscape,
  memoToCsvRow,
  exportToCsv,
  exportToNdjson,
};
