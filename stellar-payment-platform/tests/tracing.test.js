const { InMemorySpanExporter, SimpleSpanProcessor } = require('@opentelemetry/sdk-trace-base');
const { trace, context } = require('@opentelemetry/api');
const {
  initTracing,
  shutdownTracing,
  isTracingEnabled,
  forceFlush,
  getSdk,
} = require('../src/utils/tracing');

describe('OpenTelemetry Tracing Integration (Issue #285)', () => {
  const originalEnv = process.env;

  beforeEach(() => {
    process.env = { ...originalEnv };
    delete process.env.OTEL_EXPORTER_OTLP_ENDPOINT;
    delete process.env.OTEL_SERVICE_NAME;
  });

  afterEach(async () => {
    await shutdownTracing();
    process.env = originalEnv;
  });

  describe('Disabled State', () => {
    it('tracing is disabled when OTEL_EXPORTER_OTLP_ENDPOINT is not set', () => {
      delete process.env.OTEL_EXPORTER_OTLP_ENDPOINT;
      const sdk = initTracing();

      expect(sdk).toBeNull();
      expect(isTracingEnabled()).toBe(false);
      expect(getSdk()).toBeNull();
    });
  });

  describe('Enabled State & Span Emission', () => {
    it('initializes NodeSDK and emits spans when exporter is configured', async () => {
      const memoryExporter = new InMemorySpanExporter();
      const spanProcessor = new SimpleSpanProcessor(memoryExporter);

      const sdk = initTracing({
        forceEnable: true,
        endpoint: 'http://localhost:4318',
        serviceName: 'test-payment-service',
        spanProcessor,
      });

      expect(sdk).not.toBeNull();
      expect(isTracingEnabled()).toBe(true);

      // Create a test tracer and emit a root span
      const tracer = trace.getTracer('test-tracer');
      const rootSpan = tracer.startSpan('http-request-handler');
      rootSpan.setAttribute('http.method', 'GET');
      rootSpan.setAttribute('http.route', '/api/v1/federation');
      rootSpan.setAttribute('http.status_code', 200);

      // Emit a child span (simulating a Prisma database query)
      const ctx = trace.setSpan(context.active(), rootSpan);
      const childSpan = tracer.startSpan(
        'prisma:client:operation',
        {
          attributes: {
            'db.system': 'postgresql',
            'db.operation': 'user.findUnique',
            'db.name': 'stellar_tags',
          },
        },
        ctx,
      );

      childSpan.end();
      rootSpan.end();

      await forceFlush();

      // Retrieve finished spans from in-memory exporter
      const spans = memoryExporter.getFinishedSpans();
      expect(spans.length).toBe(2);

      const root = spans.find((s) => s.name === 'http-request-handler');
      const child = spans.find((s) => s.name === 'prisma:client:operation');

      expect(root).toBeDefined();
      expect(child).toBeDefined();

      // Verify parent-child correlation
      expect(child.parentSpanId).toBe(root.spanContext().spanId);
      expect(child.spanContext().traceId).toBe(root.spanContext().traceId);

      // Verify attributes
      expect(root.attributes['http.method']).toBe('GET');
      expect(root.attributes['http.status_code']).toBe(200);
      expect(child.attributes['db.system']).toBe('postgresql');
      expect(child.attributes['db.operation']).toBe('user.findUnique');
    });

    it('shuts down cleanly and clears active SDK instance', async () => {
      const memoryExporter = new InMemorySpanExporter();

      initTracing({
        forceEnable: true,
        endpoint: 'http://localhost:4318',
        exporter: memoryExporter,
      });

      expect(isTracingEnabled()).toBe(true);

      await shutdownTracing();

      expect(isTracingEnabled()).toBe(false);
      expect(getSdk()).toBeNull();
    });

    it('returns existing SDK instance if initTracing is called more than once', () => {
      const memoryExporter = new InMemorySpanExporter();

      const sdk1 = initTracing({
        forceEnable: true,
        endpoint: 'http://localhost:4318',
        exporter: memoryExporter,
      });

      const sdk2 = initTracing({
        forceEnable: true,
        endpoint: 'http://localhost:4318',
        exporter: memoryExporter,
      });

      expect(sdk1).toBe(sdk2);
    });
  });
});
