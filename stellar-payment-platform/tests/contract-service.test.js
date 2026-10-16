'use strict';

/**
 * Unit tests for contractService.js validation (#289).
 */

const { validateContractId } = require('../src/services/contractService');

describe('contractService CONTRACT_ID validation (#289)', () => {
  const validContractId = 'CDNQ7OMHIFOLZHOKWQLOGDW7CF3DRMKXJC6OULNGNBWF4O4NO2NEIGER';

  it('proceeds normally when CONTRACT_ID is valid (starts with C)', () => {
    expect(() => {
      validateContractId(validContractId);
    }).not.toThrow();
  });

  it('throws descriptive error when CONTRACT_ID is missing or empty', () => {
    expect(() => {
      validateContractId('');
    }).toThrow('CONTRACT_ID environment variable is not set');

    expect(() => {
      validateContractId(null);
    }).toThrow('CONTRACT_ID environment variable is not set');

    expect(() => {
      validateContractId(undefined);
    }).toThrow('CONTRACT_ID environment variable is not set');
  });

  it('throws descriptive error when CONTRACT_ID format is invalid (non-C prefix)', () => {
    // Starts with G (account address), not C
    expect(() => {
      validateContractId('GBRPYHIL2CI3FNQ4BXLFMNDLFJUNPU2HY3ZMFTGOBMAOTQTVHXB5TCYL');
    }).toThrow('Invalid CONTRACT_ID format: must be a valid Soroban contract address starting with C');

    // Random string
    expect(() => {
      validateContractId('INVALID_CONTRACT_123');
    }).toThrow('Invalid CONTRACT_ID format: must be a valid Soroban contract address starting with C');
  });
});
