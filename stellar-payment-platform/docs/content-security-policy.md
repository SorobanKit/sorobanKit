# Content Security Policy

This service is an API-only backend; it never serves HTML, CSS, scripts, or
media to a browser. The CSP is therefore maximally restrictive:

```
default-src 'none';
script-src 'none';
style-src 'none';
img-src 'none';
font-src 'none';
object-src 'none';
media-src 'none';
frame-ancestors 'none';
base-uri 'none';
form-action 'none';
```

## Rationale

`default-src 'none'` blocks every resource type that a browser might attempt
to load from a JSON API response interpreted as HTML (e.g. via a content-type
sniffing attack). Because no UI is served, there is no legitimate use case for
any source type, so we block them all explicitly rather than relying solely on
the default.

`frame-ancestors 'none'` is equivalent to `X-Frame-Options: DENY` and prevents
the API responses from being embedded in `<iframe>` or `<object>` elements.

## Enforcement

The policy is applied by the Helmet middleware in
`src/middleware/security.js`. It is loaded as the first middleware in the
stack in `server.js` so every response, including error responses, carries the
header.

## Tests

`tests/helmet.test.js` verifies that all directives are present on every
response, including the `/health` endpoint.
