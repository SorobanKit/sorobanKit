'use strict';

const express = require('express');
const request = require('supertest');

const mockLogger = { error: jest.fn(), warn: jest.fn(), info: jest.fn(), debug: jest.fn() };
jest.mock('../../src/logger', () => ({ logger: mockLogger }));

const { requireJson } = require('../../src/middleware/requireJson');
const { buildErrorHandler } = require('../../src/middleware/errorHandler');

const buildApp = () => {
  const app = express();
  app.use((req, res, next) => {
    req.correlationId = 'test-correlation-id';
    next();
  });

  // Attach requireJson to all routes, then route handlers.
  app.use(requireJson);
  app.get('/test', (req, res) => res.json({ method: 'GET' }));
  app.head('/test', (req, res) => res.status(200).end());
  app.delete('/test', (req, res) => res.json({ method: 'DELETE' }));
  app.post('/test', (req, res) => res.json({ method: 'POST' }));
  app.put('/test', (req, res) => res.json({ method: 'PUT' }));
  app.patch('/test', (req, res) => res.json({ method: 'PATCH' }));

  app.use(buildErrorHandler(() => false));
  return app;
};

describe('requireJson middleware', () => {
  let app;

  beforeEach(() => {
    app = buildApp();
  });

  describe('body-bearing methods (POST, PUT, PATCH)', () => {
    test('passes when Content-Type is application/json', async () => {
      const res = await request(app)
        .post('/test')
        .set('Content-Type', 'application/json')
        .send('{}');
      expect(res.status).toBe(200);
      expect(res.body.method).toBe('POST');
    });

    test('rejects when Content-Type is text/plain', async () => {
      const res = await request(app)
        .post('/test')
        .set('Content-Type', 'text/plain')
        .send('{}');
      expect(res.status).toBe(415);
    });

    test('rejects when Content-Type is application/x-www-form-urlencoded', async () => {
      const res = await request(app)
        .post('/test')
        .type('form')
        .send({ key: 'value' });
      expect(res.status).toBe(415);
    });

    test('rejects when Content-Type is missing', async () => {
      const res = await request(app)
        .post('/test')
        .send('{}');
      expect(res.status).toBe(415);
    });

    test('passes for PUT with correct Content-Type', async () => {
      const res = await request(app)
        .put('/test')
        .set('Content-Type', 'application/json')
        .send('{}');
      expect(res.status).toBe(200);
    });

    test('passes for PATCH with correct Content-Type', async () => {
      const res = await request(app)
        .patch('/test')
        .set('Content-Type', 'application/json')
        .send('{}');
      expect(res.status).toBe(200);
    });
  });

  describe('body-less methods (GET, HEAD, DELETE)', () => {
    test('GET passes without a Content-Type header', async () => {
      const res = await request(app).get('/test');
      expect(res.status).toBe(200);
      expect(res.body.method).toBe('GET');
    });

    test('HEAD passes without a Content-Type header', async () => {
      const res = await request(app).head('/test');
      expect(res.status).toBe(200);
    });

    test('DELETE passes without a Content-Type header', async () => {
      const res = await request(app).delete('/test');
      expect(res.status).toBe(200);
      expect(res.body.method).toBe('DELETE');
    });

    test('GET is not rejected even when Content-Type is set to text/plain', async () => {
      const res = await request(app)
        .get('/test')
        .set('Content-Type', 'text/plain');
      expect(res.status).toBe(200);
    });
  });
});
