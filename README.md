<p align="center">
  <img src="https://cdn.prod.website-files.com/68e09cef90d613c94c3671c0/697e805a9246c7e090054706_logo_horizontal_grey.png" alt="Yeti" width="200" />
</p>

---

# app-rate-limiter

[![Yeti](https://img.shields.io/badge/Yeti-Application-blue)](https://yetirocks.com)
[![License](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE)

> **[Yeti](https://yetirocks.com)** - The Performance Platform for Agent-Driven Development.
> Schema-driven APIs, real-time streaming, and vector search. From prompt to production.

Sliding window rate limiting with real-time abuse detection, designed for CDN edge integration.

## Features

- **4 detection conditions** evaluated per request within a configurable sliding window:
  - `high_requests` -- subscriber exceeds request count per content item
  - `high_ip_count` -- too many unique IPs accessing the same content for a subscriber
  - `multi_content` -- subscriber accessing too many distinct content items
  - `multi_session` -- multiple sessions from the same subscriber+IP
- **Configurable thresholds** per condition via RateLimitConfig table
- **CDN edge integration** via response headers (`X-Subscriber-Flagged`, `X-Subscriber-Conditions`, `X-Subscriber-Action`)
- **Single-call design** -- log request and evaluate conditions in one POST
- **Real-time streaming** via SSE and MQTT on the RequestLog table
- **24-hour TTL** on request logs for automatic cleanup

## Installation

```bash
git clone https://github.com/yetirocks/app-rate-limiter.git
cp -r app-rate-limiter ~/yeti/applications/
```

## Project Structure

```
app-rate-limiter/
  config.yaml
  schemas/
    schema.graphql
  resources/
    check.rs        # Log + evaluate in a single call
```

## Configuration

```yaml
name: "Rate Limiter"
app_id: "app-rate-limiter"
version: "0.1.0"
description: "Sliding window rate limiting with real-time piracy and abuse detection"

schemas:
  - schemas/schema.graphql

resources:
  - resources/*.rs
```

## Schema

**RequestLog** -- Logged requests with 24-hour TTL. Public read and subscribe access for real-time monitoring dashboards.

**RateLimitConfig** -- Per-host or default threshold configuration.

```graphql
type RequestLog @table(expiration: 86400, database: "app-rate-limiter")
    @export(sse: true, mqtt: true, public: [read, subscribe]) {
    id: ID! @primaryKey              # compound: subscriberId:timestamp
    subscriberId: String! @indexed
    sessionId: String @indexed
    contentName: String @indexed
    clientIp: String! @indexed
    edgeIp: String
    timestamp: String! @indexed
    method: String
    path: String
    host: String
    userAgent: String
    country: String @indexed
    metadata: String                 # JSON: arbitrary fields
}

type RateLimitConfig @table(database: "app-rate-limiter") @export {
    id: ID! @primaryKey              # "default" or per-host config
    windowSeconds: Int               # sliding window size (default 10)
    maxRequests: Int                 # requests per subscriber+content (default 50)
    maxIpCount: Int                  # unique IPs per subscriber+content (default 4)
    maxContentViews: Int             # unique content per subscriber (default 4)
    maxSessions: Int                 # sessions per subscriber+IP (default 1)
    action: String                   # "flag", "block", "monitor" (default "flag")
}
```

## API Reference

### POST /app-rate-limiter/check

Log a request and evaluate all rate-limiting conditions in a single call.

```bash
curl -X POST https://localhost:9996/app-rate-limiter/check \
  -H "Content-Type: application/json" \
  -d '{
    "subscriberId": "user-123",
    "clientIp": "203.0.113.42",
    "sessionId": "sess-abc",
    "contentName": "video-456",
    "method": "GET",
    "path": "/stream/segment-42.ts",
    "host": "cdn.example.com",
    "userAgent": "Mozilla/5.0",
    "country": "US"
  }'
```

**Response:**

```json
{
  "subscriberId": "user-123",
  "flagged": true,
  "conditions": ["high_ip_count", "multi_session"],
  "action": "flag",
  "windowLogs": 12,
  "timestamp": "1711700000"
}
```

**Response headers** (for CDN edge logic):

```
X-Subscriber-Flagged: True
X-Subscriber-Conditions: high_ip_count,multi_session
X-Subscriber-Action: flag
```

When no conditions are triggered, the response returns `flagged: false` and `X-Subscriber-Action: allow`.

### Configuring Thresholds

```bash
curl -X PUT https://localhost:9996/app-rate-limiter/RateLimitConfig/default \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{
    "id": "default",
    "windowSeconds": 30,
    "maxRequests": 100,
    "maxIpCount": 3,
    "maxContentViews": 5,
    "maxSessions": 2,
    "action": "block"
  }'
```

### Table Endpoints (auto-generated)

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/app-rate-limiter/RequestLog?limit=50` | List recent request logs |
| GET | `/app-rate-limiter/RequestLog?stream=sse` | Real-time request stream |
| GET | `/app-rate-limiter/RateLimitConfig/default` | Current threshold config |

## CDN Edge Integration

The `/check` endpoint is designed to be called from CDN edge workers (Cloudflare Workers, Akamai EdgeWorkers, AWS CloudFront Functions). The response headers allow edge logic without parsing the JSON body.

**Example Cloudflare Worker pattern:**

```javascript
async function handleRequest(request) {
  // Forward check to yeti
  const checkResponse = await fetch('https://yeti.example.com/app-rate-limiter/check', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      subscriberId: request.headers.get('X-Subscriber-Id'),
      clientIp: request.headers.get('CF-Connecting-IP'),
      sessionId: getCookie(request, 'session_id'),
      contentName: new URL(request.url).pathname,
      method: request.method,
      path: new URL(request.url).pathname,
      host: new URL(request.url).hostname,
      userAgent: request.headers.get('User-Agent'),
      country: request.headers.get('CF-IPCountry'),
    }),
  });

  const action = checkResponse.headers.get('X-Subscriber-Action');

  if (action === 'block') {
    return new Response('Access denied', { status: 403 });
  }

  // Forward to origin, passing through the flag headers
  const originResponse = await fetch(request);
  const response = new Response(originResponse.body, originResponse);
  response.headers.set('X-Subscriber-Flagged',
    checkResponse.headers.get('X-Subscriber-Flagged'));
  response.headers.set('X-Subscriber-Conditions',
    checkResponse.headers.get('X-Subscriber-Conditions'));
  return response;
}
```

---

Built with [Yeti](https://yetirocks.com) | The Performance Platform for Agent-Driven Development
