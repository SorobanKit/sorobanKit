const { NodeSDK } = require('@opentelemetry/sdk-node');
const { getNodeAutoInstrumentations } = require('@opentelemetry/auto-instrumentations-node');
const { OTLPTraceExporter } = require('@opentelemetry/exporter-trace-otlp-http');
const { PrismaInstrumentation } = require('@prisma/instrumentation');

let sdkInstance = null;

/**
 * Initializes OpenTelemetry tracing if configured or forced.
 * Tracing is disabled when OTEL_EXPORTER_OTLP_ENDPOINT is not set (unless options.forceEnable is true).
 *
 * @param {Object} options
 * @param {string} [options.endpoint] - Custom OTLP collector endpoint.
 * @param {string} [options.serviceName] - Custom service name for trace spans.
 * @param {Object} [options.exporter] - Custom trace exporter instance (e.g. InMemorySpanExporter for testing).
 * @param {boolean} [options.forceEnable] - Force tracing enabled even if env var is missing.
 * @returns {NodeSDK|null} The initialized NodeSDK instance or null if disabled.
 */
function initTracing(options = {}) {
  // If an SDK is already active, return the existing instance
  if (sdkInstance) {
    return sdkInstance;
  }

  const endpoint = options.endpoint || process.env.OTEL_EXPORTER_OTLP_ENDPOINT;
  const isEnabled = options.forceEnable || Boolean(endpoint);

  if (!isEnabled) {
    // Acceptance criteria: Tracing is disabled when OTEL_EXPORTER_OTLP_ENDPOINT is not set
    return null;
  }

  const serviceName =
    options.serviceName ||
    process.env.OTEL_SERVICE_NAME ||
    'stellar-payment-platform';

  // Format OTLP HTTP traces endpoint if not provided via custom exporter
  let traceExporter = options.exporter;
  if (!traceExporter && endpoint) {
    const formattedUrl = endpoint.endsWith('/v1/traces')
      ? endpoint
      : `${endpoint.replace(/\/+$/, '')}/v1/traces`;

    traceExporter = new OTLPTraceExporter({
      url: formattedUrl,
    });
  }

  const sdkConfig = {
    serviceName,
    instrumentations: [
      getNodeAutoInstrumentations({
        '@opentelemetry/instrumentation-express': { enabled: true },
        '@opentelemetry/instrumentation-http': { enabled: true },
        '@opentelemetry/instrumentation-fs': { enabled: false },
      }),
      new PrismaInstrumentation(),
    ],
  };

  if (options.spanProcessor) {
    sdkConfig.spanProcessor = options.spanProcessor;
  } else if (options.spanProcessors) {
    sdkConfig.spanProcessors = options.spanProcessors;
  } else if (traceExporter) {
    sdkConfig.traceExporter = traceExporter;
  }

  const sdk = new NodeSDK(sdkConfig);

  try {
    sdk.start();
    sdkInstance = sdk;
    return sdkInstance;
  } catch (error) {
    console.error('Failed to start OpenTelemetry NodeSDK:', error);
    return null;
  }
}

/**
 * Shuts down the active OpenTelemetry SDK instance cleanly.
 * @returns {Promise<void>}
 */
function shutdownTracing() {
  if (sdkInstance) {
    return sdkInstance
      .shutdown()
      .then(() => {
        sdkInstance = null;
      })
      .catch((error) => {
        console.error('Error shutting down OpenTelemetry SDK:', error);
        sdkInstance = null;
      });
  }
  return Promise.resolve();
}

/**
 * Checks if tracing is currently active.
 * @returns {boolean}
 */
function isTracingEnabled() {
  return sdkInstance !== null;
}

/**
 * Forces an immediate flush of all batched spans.
 * Useful in serverless handlers, tests, or before shutdown.
 * @returns {Promise<void>}
 */
function forceFlush() {
  if (
    sdkInstance &&
    sdkInstance._tracerProvider &&
    typeof sdkInstance._tracerProvider.forceFlush === 'function'
  ) {
    return sdkInstance._tracerProvider.forceFlush();
  }
  return Promise.resolve();
}

// Auto-initialize when required if OTEL_EXPORTER_OTLP_ENDPOINT is configured in process.env
if (process.env.OTEL_EXPORTER_OTLP_ENDPOINT) {
  initTracing();
}

process.on('SIGTERM', () => {
  shutdownTracing()
    .then(() => console.log('Tracing terminated'))
    .catch((error) => console.error('Error terminating tracing', error));
});

module.exports = {
  initTracing,
  shutdownTracing,
  isTracingEnabled,
  forceFlush,
  getSdk: () => sdkInstance,
};
