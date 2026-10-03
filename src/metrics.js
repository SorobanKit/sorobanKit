'use strict';

const client = require('prom-client');

const register = new client.Registry();

client.collectDefaultMetrics({ register });

const httpRequestsTotal = new client.Counter({
  name: 'http_requests_total',
  help: 'Total number of HTTP requests',
  labelNames: ['method', 'route', 'status'],
  registers: [register],
});

const httpRequestDurationSeconds = new client.Histogram({
  name: 'http_request_duration_seconds',
  help: 'HTTP request duration in seconds',
  labelNames: ['method', 'route', 'status'],
  buckets: [0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1, 2.5, 5, 10],
  registers: [register],
});

const webhookDeliveriesTotal = new client.Counter({
  name: 'webhook_deliveries_total',
  help: 'Total number of webhook delivery attempts by outcome',
  labelNames: ['status'],
  registers: [register],
});

const webhookDeliveryDurationSeconds = new client.Histogram({
  name: 'webhook_delivery_duration_seconds',
  help: 'Webhook delivery duration in seconds',
  labelNames: ['status'],
  buckets: [0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1, 2.5, 5, 10],
  registers: [register],
});

const webhookDlqSize = new client.Gauge({
  name: 'webhook_dlq_size',
  help: 'Current number of webhook deliveries in the dead-letter queue',
  registers: [register],
});

function observeHttpRequest({ method, route, status, durationSeconds }) {
  const labels = { method, route, status: String(status) };
  httpRequestsTotal.inc(labels);
  httpRequestDurationSeconds.observe(labels, durationSeconds);
}

function observeWebhookDelivery({ status, durationSeconds }) {
  const labels = { status };
  webhookDeliveriesTotal.inc(labels);
  if (typeof durationSeconds === 'number') {
    webhookDeliveryDurationSeconds.observe(labels, durationSeconds);
  }
}

function setWebhookDlqSize(size) {
  webhookDlqSize.set(size);
}

module.exports = {
  register,
  httpRequestsTotal,
  httpRequestDurationSeconds,
  webhookDeliveriesTotal,
  webhookDeliveryDurationSeconds,
  webhookDlqSize,
  observeHttpRequest,
  observeWebhookDelivery,
  setWebhookDlqSize,
};
