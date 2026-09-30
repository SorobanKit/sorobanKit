# Webhook Signature Verification

Every webhook POST sent by Stellar Tags includes an HMAC-SHA256 signature so
merchants can confirm the request genuinely came from the platform and has not
been tampered with in transit.

## Headers

| Header | Description |
|---|---|
| `X-Webhook-Signature` | Hex-encoded HMAC-SHA256 of `"<unix_ts>.<raw JSON body>"`, signed with the webhook secret. |
| `X-Stellar-Tags-Signature` | Alias for `X-Webhook-Signature` — kept for backward compatibility. |
| `X-Webhook-Timestamp` | Unix timestamp (seconds since epoch) of the delivery. Included in the signed string to prevent replay attacks. |

> Prefer `X-Webhook-Signature` for new integrations.

## How the signature is computed

```
signed_string = unix_timestamp + "." + raw_JSON_body
HMAC-SHA256( key=<webhook_secret>, message=<signed_string> )
```

`unix_timestamp` is the value of the `X-Webhook-Timestamp` header (an integer number
of seconds since the Unix epoch).  The raw JSON body is the exact byte sequence sent
over the wire.  The webhook secret is the value you supplied when registering your
webhook URL.

## Test verification endpoint

To validate a payload and signature before wiring up production code, send the
payload and the secret to `POST /api/v1/webhooks/verify-test` and include the
signature in the `X-Webhook-Signature` header. Because signatures are computed
over the raw body bytes, pass the payload as a JSON **string** exactly as it was
sent over the wire:

```bash
curl -X POST https://api.stellar-tags.example/api/v1/webhooks/verify-test \
  -H 'Content-Type: application/json' \
  -H 'X-Webhook-Signature: <hex-signature>' \
  -d '{
    "secret": "your_webhook_secret",
    "payload": "{\"event\":\"payment.created\",\"id\":\"evt_123\",\"amount\":42}"
  }'
```

A successful response looks like:

```json
{
  "ok": true,
  "valid": true,
  "message": "Webhook signature verification succeeded.",
  "expectedSignature": "<hex-signature>",
  "receivedSignature": "<hex-signature>"
}
```

When the signature is wrong, the endpoint responds with `401` and a detailed
error payload including the expected and received values.

## Verifying in Node.js

```js
const crypto = require('crypto');

/**
 * Returns true when the timestamp + body match the signature.
 *
 * @param {string} secret      - The webhook secret you registered.
 * @param {string} timestamp   - Value of the X-Webhook-Timestamp header.
 * @param {string} rawBody     - The raw request body (Buffer or string).
 * @param {string} sigHeader   - Value of the X-Webhook-Signature header.
 */
function verifySignature(secret, timestamp, rawBody, sigHeader) {
  const signedString = `${timestamp}.${rawBody}`;
  const expected = crypto
    .createHmac('sha256', secret)
    .update(signedString)
    .digest('hex');

  // Constant-time comparison prevents timing-oracle attacks.
  return crypto.timingSafeEqual(
    Buffer.from(expected, 'hex'),
    Buffer.from(sigHeader, 'hex'),
  );
}

// Express example ─ use express.raw() to keep the body as a Buffer.
app.post('/webhook', express.raw({ type: 'application/json' }), (req, res) => {
  const sig = req.headers['x-webhook-signature'];
  const ts = req.headers['x-webhook-timestamp'];

  // Reject deliveries older than 5 minutes to prevent replay attacks.
  if (!ts || Math.abs(Date.now() / 1000 - Number(ts)) > 300) {
    return res.status(401).json({ error: 'Timestamp missing or too old' });
  }
  if (!sig || !verifySignature(process.env.WEBHOOK_SECRET, ts, req.body, sig)) {
    return res.status(401).json({ error: 'Invalid signature' });
  }

  const payload = JSON.parse(req.body.toString());
  console.log('Verified webhook event:', payload.event);
  res.sendStatus(200);
});
```

## Verifying in Python

```python
import hashlib
import hmac
import json
import time
from flask import Flask, request, abort

app = Flask(__name__)
WEBHOOK_SECRET = b"your_webhook_secret"

@app.route("/webhook", methods=["POST"])
def webhook():
    raw_body = request.get_data()  # keep raw bytes before parsing
    sig = request.headers.get("X-Webhook-Signature", "")
    ts = request.headers.get("X-Webhook-Timestamp", "")

    # Reject deliveries older than 5 minutes.
    if not ts or abs(time.time() - int(ts)) > 300:
        abort(401, "Timestamp missing or too old")

    signed_string = f"{ts}.".encode() + raw_body
    expected = hmac.new(WEBHOOK_SECRET, signed_string, hashlib.sha256).hexdigest()
    # hmac.compare_digest performs a constant-time comparison, preventing
    # timing attacks that could otherwise leak the expected signature byte by byte.
    if not hmac.compare_digest(expected, sig):
        abort(401, "Invalid signature")

    payload = json.loads(raw_body)
    print("Verified event:", payload["event"])
    return "", 200
```

## Verifying in Go

```go
package main

import (
    "crypto/hmac"
    "crypto/sha256"
    "encoding/hex"
    "fmt"
    "io"
    "net/http"
    "strconv"
    "time"
)

func verifySignature(secret []byte, timestamp, rawBody []byte, sigHeader string) bool {
    signed := append(timestamp, '.')
    signed = append(signed, rawBody...)
    mac := hmac.New(sha256.New, secret)
    mac.Write(signed)
    expected := hex.EncodeToString(mac.Sum(nil))
    // hmac.Equal performs a constant-time comparison, preventing timing
    // attacks that could otherwise leak the expected signature byte by byte.
    // A plain == comparison is not constant-time and must not be used here.
    return hmac.Equal([]byte(expected), []byte(sigHeader))
}

func webhookHandler(w http.ResponseWriter, r *http.Request) {
    body, _ := io.ReadAll(r.Body)
    sig := r.Header.Get("X-Webhook-Signature")
    tsStr := r.Header.Get("X-Webhook-Timestamp")

    ts, err := strconv.ParseInt(tsStr, 10, 64)
    if err != nil || abs64(time.Now().Unix()-ts) > 300 {
        http.Error(w, "Timestamp missing or too old", http.StatusUnauthorized)
        return
    }

    if !verifySignature([]byte("your_webhook_secret"), []byte(tsStr), body, sig) {
        http.Error(w, "Invalid signature", http.StatusUnauthorized)
        return
    }
    // process payload ...
    w.WriteHeader(http.StatusOK)
}

func abs64(n int64) int64 {
    if n < 0 { return -n }
    return n
}
```

## Security recommendations

- **Always verify** the signature before trusting the payload.
- Use **`timingSafeEqual`** (or `hmac.compare_digest` in Python, `hmac.Equal`
  in Go) — regular string equality is vulnerable to timing attacks.
- Rotate your webhook secret immediately if you suspect it has been leaked.
- **Always reject** requests whose `X-Webhook-Timestamp` is more than five
  minutes in the past — the timestamp is now part of the signature so replayed
  requests will fail verification after the window closes.

## Replay-attack guard

Because `X-Webhook-Timestamp` is now part of the signed string, an attacker who
captures a valid delivery cannot reuse it after the tolerance window has passed —
the timestamp in the captured request will be too old and the signature will no
longer match a freshly-computed one using the current time.

Reject deliveries whose `X-Webhook-Timestamp` differs from the current time by
more than five minutes:

```js
app.post('/webhook', express.raw({ type: 'application/json' }), (req, res) => {
  const sig = req.headers['x-webhook-signature'];
  const ts = req.headers['x-webhook-timestamp'];

  if (!ts || Math.abs(Date.now() / 1000 - Number(ts)) > 300) {
    return res.status(401).json({ error: 'Timestamp missing or too old — possible replay attack' });
  }
  if (!verifySignature(process.env.WEBHOOK_SECRET, ts, req.body, sig)) {
    return res.status(401).json({ error: 'Invalid signature' });
  }

  const payload = JSON.parse(req.body.toString());
  console.log('Verified webhook event:', payload.event);
  res.sendStatus(200);
});
```
