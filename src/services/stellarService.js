'use strict';

const axios = require('axios');
const { logger } = require('../utils/logger');
const { withDeadline } = require('../utils/deadline');
const { getBreaker } = require('../utils/circuitBreaker');
const { getBulkhead } = require('../utils/bulkhead');

const HORIZON_URL = process.env.HORIZON_URL || 'https://horizon-testnet.stellar.org';
const NETWORK_LABEL = process.env.STELLAR_NETWORK || 'default';
const RETRY_MAX_ATTEMPTS = 3;
const RETRY_BASE_DELAY_MS = 500;

function isTransientHorizonError(err) {
  if (!err) return false;
  const status = err.response && err.response.status;
  if (status === 503) return true;
  // Network timeouts / connection failures have no response.
  if (!err.response) {
    return (
      err.code === 'ECONNABORTED' ||
      err.code === 'ETIMEDOUT' ||
      err.code === 'ECONNRESET' ||
      err.code === 'ENOTFOUND' ||
      err.code === 'EAI_AGAIN'
    );
  }
  return false;
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function retryHorizonRequest(fn, maxAttempts = RETRY_MAX_ATTEMPTS, signal = null) {
  const breaker = getBreaker(NETWORK_LABEL);
  const bulkhead = getBulkhead(NETWORK_LABEL);

  return bulkhead.execute(() =>
    breaker.execute(async () => {
      let lastError;
      for (let attempt = 1; attempt <= maxAttempts; attempt += 1) {
        try {
          return await withDeadline(fn(), signal);
        } catch (err) {
          lastError = err;
          if (!isTransientHorizonError(err) || attempt === maxAttempts) {
            throw err;
          }
          const delay = RETRY_BASE_DELAY_MS * Math.pow(2, attempt - 1);
          logger.warn(
            { attempt, maxAttempts, delay, status: err.response && err.response.status, network: NETWORK_LABEL },
            'Transient Horizon error, retrying'
          );
          await sleep(delay);
        }
      }
      throw lastError;
    })
  );
}

async function getAccount(accountId, signal = null) {
  return retryHorizonRequest(async () => {
    const response = await axios.get(`${HORIZON_URL}/accounts/${accountId}`);
    return response.data;
  }, RETRY_MAX_ATTEMPTS, signal);
}

async function getAccountBalance(accountId, signal = null) {
  return retryHorizonRequest(async () => {
    const response = await axios.get(`${HORIZON_URL}/accounts/${accountId}`);
    const balances = response.data.balances || [];
    const native = balances.find((b) => b.asset_type === 'native');
    return native ? native.balance : '0';
  }, RETRY_MAX_ATTEMPTS, signal);
}

async function getTransactions(accountId, limit = 10, signal = null) {
  return retryHorizonRequest(async () => {
    const response = await axios.get(`${HORIZON_URL}/accounts/${accountId}/transactions`, {
      params: { limit, order: 'desc' },
    });
    return response.data._embedded && response.data._embedded.records
      ? response.data._embedded.records
      : [];
  }, RETRY_MAX_ATTEMPTS, signal);
}

async function submitTransaction(envelopeXdr, signal = null) {
  return retryHorizonRequest(async () => {
    const response = await axios.post(
      `${HORIZON_URL}/transactions`,
      `tx=${encodeURIComponent(envelopeXdr)}`,
      { headers: { 'Content-Type': 'application/x-www-form-urlencoded' } }
    );
    return response.data;
  }, RETRY_MAX_ATTEMPTS, signal);
}

module.exports = {
  retryHorizonRequest,
  isTransientHorizonError,
  getAccount,
  getAccountBalance,
  getTransactions,
  submitTransaction,
};
