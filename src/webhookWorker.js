'use strict';

const { WebhookEvent } = require('./models');
const { deliverWebhook } = require('./webhookDelivery');
const {
  webhookDeliveriesTotal,
  webhookDeliveryDurationSeconds,
  webhookDlqSize,
} = require('./metrics');

const MAX_ATTEMPTS = 5;
const BASE_BACKOFF_MS = 1000;
const RETRYABLE_STATUS_CODES = new Set([429, 500, 502, 503, 504]);

function isRetryableStatus(status) {
  return RETRYABLE_STATUS_CODES.has(status);
}

function backoffDelayMs(attempt) {
  return BASE_BACKOFF_MS * Math.pow(2, attempt - 1);
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function processWebhookEvent(event) {
  let attempt = 0;
  let lastError = null;

  while (attempt < MAX_ATTEMPTS) {
    attempt += 1;

    let response;
    const deliveryStart = process.hrtime.bigint();
    try {
      response = await deliverWebhook(event);
    } catch (err) {
      const durationSeconds = Number(process.hrtime.bigint() - deliveryStart) / 1e9;
      webhookDeliveryDurationSeconds.observe(durationSeconds);
      lastError = err && err.message ? err.message : String(err);
      await event.update({ attempt_count: attempt, last_error: lastError });
      if (attempt < MAX_ATTEMPTS) {
        await sleep(backoffDelayMs(attempt));
        continue;
      }
      break;
    }

    const durationSeconds = Number(process.hrtime.bigint() - deliveryStart) / 1e9;
    webhookDeliveryDurationSeconds.observe(durationSeconds);

    if (response && response.status >= 200 && response.status < 300) {
      await event.update({
        status: 'delivered',
        attempt_count: attempt,
        last_error: null,
        delivered_at: new Date(),
      });
      webhookDeliveriesTotal.inc({ status: 'success' });
      return event;
    }

    const status = response ? response.status : null;
    lastError = `HTTP ${status}`;
    await event.update({ attempt_count: attempt, last_error: lastError });

    if (!isRetryableStatus(status)) {
      break;
    }

    if (attempt < MAX_ATTEMPTS) {
      await sleep(backoffDelayMs(attempt));
    }
  }

  await event.update({
    status: 'failed',
    attempt_count: attempt,
    last_error: lastError,
  });
  webhookDeliveriesTotal.inc({ status: 'failure' });
  await moveToDeadLetterQueue(event);
  return event;
}

async function moveToDeadLetterQueue(event) {
  await WebhookEvent.create({
    event_type: event.event_type,
    payload: event.payload,
    merchant_id: event.merchant_id,
    status: 'dead_letter',
    attempt_count: event.attempt_count,
    last_error: event.last_error,
    original_event_id: event.id,
  });
  const dlqDepth = await WebhookEvent.count({ where: { status: 'dead_letter' } });
  webhookDlqSize.set(dlqDepth);
}

module.exports = {
  processWebhookEvent,
  isRetryableStatus,
  backoffDelayMs,
  MAX_ATTEMPTS,
  RETRYABLE_STATUS_CODES,
};
