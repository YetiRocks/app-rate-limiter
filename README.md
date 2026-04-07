<p align="center">
  <img src="https://cdn.prod.website-files.com/68e09cef90d613c94c3671c0/697e805a9246c7e090054706_logo_horizontal_grey.png" alt="Yeti" width="200" />
</p>

---

# app-rate-limiter

[![Yeti](https://img.shields.io/badge/Yeti-Application-blue)](https://yetirocks.com)
[![License](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE)

> **[Yeti](https://yetirocks.com)** - The Performance Platform for Agent-Driven Development.
> Schema-driven APIs, real-time streaming, and vector search. From prompt to production.

**Sliding window rate limiting at the edge.** One POST, four abuse conditions, sub-millisecond decisions.

Rate limiting at the CDN edge requires sub-millisecond decisions on every request. Bolting a standalone rate limiter onto your CDN means another service to deploy, another network hop to absorb, and another failure mode to handle. app-rate-limiter collapses all of that into a single yeti application: log a request, evaluate four abuse conditions against a sliding window, and return the verdict in response headers that edge workers can act on without parsing JSON. No external dependencies. No separate infrastructure. No cloud rate-limiting service. One application, one POST, one response.

---

## Why app-rate-limiter

CDN edge workers need to make enforce-or-allow decisions before forwarding a request to origin. Building that capability from scratch means standing up a rate-limiting service, a time-series store for request logs, a configuration layer for thresholds, and a real-time monitoring feed -- four moving parts for one capability.

app-rate-limiter collapses all of that into a single yeti application:

- **Single-call design** -- log a request and evaluate all conditions in one `POST /check`. No multi-step workflows, no pre-check then post-check.
- **Header-driven verdicts** -- `X-Subscriber-Flagged`, `X-Subscriber-Conditions`, and `X-Subscriber-Action` headers let edge workers act on the result without parsing the response body.
- **Configurable sliding window** -- window size, thresholds, and enforcement action are all stored in a `RateLimitConfig` table and changeable at runtime via REST.
- **Four detection conditions** -- high request volume, IP proliferation, content breadth scanning, and session multiplexing. Each independently configurable.
- **24-hour auto-expiry** -- request logs expire after 86400 seconds. No cleanup jobs, no storage growth.
- **Real-time streaming** -- SSE and MQTT on the RequestLog table for live monitoring dashboards and alerting pipelines.
- **Single binary deployment** -- compiles into a native Rust plugin. No Node.js, no npm, no Docker compose. Loads with yeti in seconds.

---

## Quick Start

### 1. Install

```bash
cd ~/yeti/applications
git clone https://github.com/yetirocks/app-rate-limiter.git
```

Restart yeti. The application compiles automatically on first load (~2 minutes) and is cached for subsequent starts (~10 seconds).

### 2. Configure thresholds

```bash
curl -X PUT https://localhost:9996/app-rate-limiter/api/RateLimitConfig/default \
  -H "Content-Type: application/json" \
  -d '{
    "id": "default",
    "windowSeconds": 10,
    "maxRequests": 50,
    "maxIpCount": 4,
    "maxContentViews": 4,
    "maxSessions": 1,
    "action": "flag"
  }'
```

Response:

```json
{
  "id": "default",
  "windowSeconds": 10,
  "maxRequests": 50,
  "maxIpCount": 4,
  "maxContentViews": 4,
  "maxSessions": 1,
  "action": "flag"
}
```

### 3. Send a request for evaluation

```bash
curl -X POST https://localhost:9996/app-rate-limiter/api/check \
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

Response:

```json
{
  "subscriberId": "user-123",
  "flagged": false,
  "conditions": [],
  "action": "allow",
  "windowLogs": 0,
  "timestamp": "1711700000"
}
```

Response headers:

```
X-Subscriber-Flagged: False
X-Subscriber-Conditions:
X-Subscriber-Action: allow
```

### 4. Trigger a condition

Send the same request from multiple IPs to exceed the `maxIpCount` threshold (default 4):

```bash
for ip in 203.0.113.1 203.0.113.2 203.0.113.3 203.0.113.4 203.0.113.5; do
  curl -s -X POST https://localhost:9996/app-rate-limiter/api/check \
    -H "Content-Type: application/json" \
    -d "{
      \"subscriberId\": \"user-123\",
      \"clientIp\": \"$ip\",
      \"sessionId\": \"sess-abc\",
      \"contentName\": \"video-456\"
    }" | python3 -m json.tool
done
```

The fifth request returns:

```json
{
  "subscriberId": "user-123",
  "flagged": true,
  "conditions": ["high_ip_count"],
  "action": "flag",
  "windowLogs": 4,
  "timestamp": "1711700005"
}
```

### 5. Watch the real-time stream

```bash
# SSE stream -- see every request as it is logged
curl --max-time 30 "https://localhost:9996/app-rate-limiter/api/RequestLog?stream=sse"

# MQTT -- subscribe to request log changes
mosquitto_sub -t "app-rate-limiter/RequestLog" -h localhost -p 8883
```

---

## Architecture

```
CDN Edge Worker (Cloudflare, Akamai, CloudFront)
    |
    |  POST /app-rate-limiter/api/check
    |  { subscriberId, clientIp, sessionId, contentName, ... }
    |
    v
+------------------------------------------------------------------+
|                       app-rate-limiter                            |
|                                                                  |
|  1. Write RequestLog   +--------------+                          |
|     (24h TTL)   -----> | RequestLog   |                          |
|                        | (RocksDB)    |                          |
|  2. Load config        +--------------+                          |
|     (per-host or  ---> | RateLimitConfig |                       |
|      "default")        +-----------------+                       |
|                                                                  |
|  3. Sliding window query: all logs for subscriberId              |
|     within windowSeconds                                         |
|                                                                  |
|  4. Evaluate 4 conditions:                                       |
|     +------------------+-------------------+                     |
|     | high_requests    | high_ip_count     |                     |
|     | multi_content    | multi_session     |                     |
|     +------------------+-------------------+                     |
|                                                                  |
|  5. Return verdict via headers + JSON body                       |
+------------------------------------------------------------------+
    |
    |  Response headers:
    |    X-Subscriber-Flagged: True
    |    X-Subscriber-Conditions: high_ip_count,multi_session
    |    X-Subscriber-Action: flag
    |
    v
CDN Edge Worker enforces (block, flag, allow)
    |
    +-------> SSE / MQTT subscribers (dashboards, alerts)
```

**Write path:** Edge request -> `POST /check` -> write RequestLog with 24h TTL -> load RateLimitConfig -> sliding window query for subscriber -> evaluate 4 conditions -> return verdict in headers + JSON.

**Read path:** Dashboard or alerting system subscribes to `RequestLog?stream=sse` or MQTT topic `app-rate-limiter/RequestLog` for real-time visibility. REST queries with `?limit=N` for historical analysis.

---

## Features

### Request Check (POST /app-rate-limiter/api/check)

Log a request and evaluate all rate-limiting conditions in a single call. This is the primary endpoint for CDN edge integration.

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `subscriberId` | String | Yes | Subscriber or user identifier |
| `clientIp` | String | Yes | Client IP address (from edge, e.g. `CF-Connecting-IP`) |
| `sessionId` | String | No | Session identifier for multi-session detection |
| `contentName` | String | No | Content item identifier (video ID, asset path, etc.) |
| `edgeIp` | String | No | Edge server IP (for diagnostics) |
| `method` | String | No | HTTP method (default: "GET") |
| `path` | String | No | Request path |
| `host` | String | No | Request hostname |
| `userAgent` | String | No | User-Agent header value |
| `country` | String | No | Country code (from edge geo, e.g. `CF-IPCountry`) |
| `metadata` | String (JSON) | No | Arbitrary key-value pairs |

### Detection Conditions

Four conditions are evaluated on every request. Each compares a count within the sliding window against a configurable threshold:

| Condition | What it detects | Default threshold | How it works |
|-----------|----------------|-------------------|--------------|
| `high_requests` | Request flooding | 50 requests | Counts requests from same subscriber for same content item within window |
| `high_ip_count` | Credential sharing | 4 unique IPs | Counts distinct client IPs accessing the same content for a subscriber |
| `multi_content` | Content scraping | 4 content items | Counts distinct content items accessed by a subscriber |
| `multi_session` | Session cloning | 1 session | Counts distinct sessions from the same subscriber + IP combination |

A request is flagged when **any** condition exceeds its threshold. Multiple conditions can trigger simultaneously.

### Response Headers

Response headers allow edge workers to act on the verdict without parsing the JSON body:

| Header | Values | Description |
|--------|--------|-------------|
| `X-Subscriber-Flagged` | `True` / `False` | Whether any condition was triggered |
| `X-Subscriber-Conditions` | Comma-separated list | Which conditions fired (e.g. `high_ip_count,multi_session`) |
| `X-Subscriber-Action` | `flag` / `block` / `monitor` / `allow` | Configured action when flagged; `allow` when clean |

### Response Body

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

| Field | Type | Description |
|-------|------|-------------|
| `subscriberId` | String | Echo of the input subscriber ID |
| `flagged` | Boolean | `true` if any condition exceeded its threshold |
| `conditions` | Array | List of triggered condition names |
| `action` | String | `"allow"` when clean, or the configured action (`"flag"`, `"block"`, `"monitor"`) |
| `windowLogs` | Number | Count of existing logs found in the sliding window (excluding the current request) |
| `timestamp` | String | Unix timestamp of the logged request |

### Sliding Window

The sliding window determines how far back the condition evaluation looks. All four conditions use the same window.

- **Default:** 10 seconds
- **Configurable:** Set `windowSeconds` on the `RateLimitConfig` record
- **How it works:** On each `/check` call, the resource queries all `RequestLog` entries for the subscriber where `timestamp >= (now - windowSeconds)`. The current request is excluded from the count to avoid off-by-one self-triggering.
- **Auto-expiry:** RequestLog records have a 24-hour TTL (86400 seconds). Records expire automatically from RocksDB -- no cleanup cron, no storage growth.

### Real-Time Streaming (auto-generated)

Real-time updates are built into the platform via `@export(sse: true, mqtt: true)` on the RequestLog table:

```bash
# SSE -- server-sent events for dashboards
GET /app-rate-limiter/api/RequestLog?stream=sse

# MQTT -- subscribe to request log changes
mosquitto_sub -t "app-rate-limiter/RequestLog" -h localhost -p 8883
```

Every request logged via `/check` is written to the RequestLog table and immediately broadcast to all SSE and MQTT subscribers. Use this for real-time abuse dashboards, alerting pipelines, or SIEM integration.

### REST CRUD (auto-generated)

Full CRUD on all tables is auto-generated from the schema:

| Endpoint | Methods | Description |
|----------|---------|-------------|
| `/app-rate-limiter/api/RequestLog` | GET, POST | List/create request logs |
| `/app-rate-limiter/api/RequestLog/{id}` | GET, PUT, DELETE | Read/update/delete a request log |
| `/app-rate-limiter/api/RateLimitConfig` | GET, POST | List/create config entries |
| `/app-rate-limiter/api/RateLimitConfig/{id}` | GET, PUT, DELETE | Read/update/delete a config entry |

Query parameters:

| Parameter | Example | Description |
|-----------|---------|-------------|
| `limit` | `?limit=50` | Limit number of results |
| `stream` | `?stream=sse` | Switch to SSE streaming mode |

---

## Data Model

### RequestLog Table

Logged requests with a 24-hour TTL. Public read and subscribe access for real-time monitoring dashboards.

| Field | Type | Indexed | Description |
|-------|------|---------|-------------|
| `id` | ID! | Primary key | Compound key: `subscriberId:timestamp` |
| `subscriberId` | String! | Yes | Subscriber or user identifier |
| `sessionId` | String | Yes | Session identifier |
| `contentName` | String | Yes | Content item identifier |
| `clientIp` | String! | Yes | Client IP address |
| `edgeIp` | String | -- | Edge server IP |
| `timestamp` | String! | Yes | Unix timestamp of the request |
| `method` | String | -- | HTTP method |
| `path` | String | -- | Request path |
| `host` | String | -- | Request hostname |
| `userAgent` | String | -- | User-Agent header |
| `country` | String | Yes | Country code |
| `metadata` | String | -- | Arbitrary JSON metadata |

**Expiration:** 86400 seconds (24 hours). Records are automatically removed by RocksDB after expiry.

**Public access:** `read` and `subscribe` are public (no authentication required). This allows monitoring dashboards to consume the stream without credentials.

### RateLimitConfig Table

Per-host or default threshold configuration. Authenticated access only.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `id` | ID! | -- | `"default"` or a per-host key (e.g. `"cdn.example.com"`) |
| `windowSeconds` | Int | 10 | Sliding window size in seconds |
| `maxRequests` | Int | 50 | Max requests per subscriber + content item within window |
| `maxIpCount` | Int | 4 | Max unique IPs per subscriber + content item within window |
| `maxContentViews` | Int | 4 | Max unique content items per subscriber within window |
| `maxSessions` | Int | 1 | Max unique sessions per subscriber + IP within window |
| `action` | String | `"flag"` | Enforcement action: `"flag"`, `"block"`, or `"monitor"` |

**Actions:**

| Action | Behavior |
|--------|----------|
| `flag` | Set `X-Subscriber-Flagged: True` but allow the request. Edge worker decides enforcement. |
| `block` | Signal the edge worker to deny the request (e.g. return 403). |
| `monitor` | Log the condition but take no enforcement action. For observation-only deployments. |

---

## Configuration

### Setting thresholds via REST

Create or update the default configuration:

```bash
curl -X PUT https://localhost:9996/app-rate-limiter/api/RateLimitConfig/default \
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

The `/check` endpoint reads the `"default"` config record on every call. Changes take effect immediately -- no restart required.

### Per-host configuration

Store a config record keyed by hostname to override defaults for specific CDN origins. The current implementation reads the `"default"` key; extend the check resource to look up `host`-specific configs for multi-tenant deployments.

### config.yaml

```yaml
name: "Rate Limiter"
app_id: "app-rate-limiter"
version: "0.1.0"
description: "Sliding window rate limiting with real-time piracy and abuse detection"

schemas:
  path: schemas/schema.graphql

resources:
  path: resources/*.rs
  route: /api
```

### Project Structure

```
app-rate-limiter/
  config.yaml              # App configuration
  schemas/
    schema.graphql         # RequestLog + RateLimitConfig tables
  resources/
    check.rs               # Log + evaluate in a single call
```

---

## CDN Edge Integration

The `/check` endpoint is designed to be called from CDN edge workers. Response headers communicate the verdict so edge logic can act without parsing the JSON body.

### Cloudflare Worker

```javascript
export default {
  async fetch(request, env) {
    const url = new URL(request.url);

    // Forward request metadata to rate limiter
    const checkResponse = await fetch(`${env.YETI_ORIGIN}/app-rate-limiter/api/check`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        subscriberId: request.headers.get('X-Subscriber-Id') || 'anonymous',
        clientIp: request.headers.get('CF-Connecting-IP'),
        sessionId: getCookie(request, 'session_id'),
        contentName: url.pathname,
        method: request.method,
        path: url.pathname,
        host: url.hostname,
        userAgent: request.headers.get('User-Agent'),
        country: request.headers.get('CF-IPCountry'),
      }),
    });

    // Read the verdict from headers
    const action = checkResponse.headers.get('X-Subscriber-Action');
    const flagged = checkResponse.headers.get('X-Subscriber-Flagged');
    const conditions = checkResponse.headers.get('X-Subscriber-Conditions');

    // Enforce: block if the limiter says so
    if (action === 'block') {
      return new Response('Access denied', {
        status: 403,
        headers: {
          'X-Subscriber-Flagged': flagged,
          'X-Subscriber-Conditions': conditions,
        },
      });
    }

    // Pass through to origin, forwarding flag headers for logging
    const originResponse = await fetch(request);
    const response = new Response(originResponse.body, originResponse);
    response.headers.set('X-Subscriber-Flagged', flagged);
    response.headers.set('X-Subscriber-Conditions', conditions);
    response.headers.set('X-Subscriber-Action', action);
    return response;
  },
};

function getCookie(request, name) {
  const cookie = request.headers.get('Cookie') || '';
  const match = cookie.match(new RegExp(`${name}=([^;]+)`));
  return match ? match[1] : '';
}
```

### Akamai EdgeWorker

```javascript
import { httpRequest } from 'http-request';
import { createResponse } from 'create-response';
import { Cookies } from 'cookies';

export async function onClientRequest(request) {
  const cookies = new Cookies(request.getHeader('Cookie'));
  const sessionId = cookies.get('session_id') || '';

  const checkResponse = await httpRequest(`${YETI_ORIGIN}/app-rate-limiter/api/check`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      subscriberId: request.getHeader('X-Subscriber-Id')[0] || 'anonymous',
      clientIp: request.getHeader('True-Client-IP')[0],
      sessionId: sessionId,
      contentName: request.path,
      method: request.method,
      path: request.path,
      host: request.host,
      userAgent: request.getHeader('User-Agent')[0],
      country: request.getHeader('X-Akamai-Edgescape-Country')[0] || '',
    }),
  });

  const action = checkResponse.getHeader('X-Subscriber-Action')[0];

  if (action === 'block') {
    request.respondWith(
      createResponse(403, { 'Content-Type': 'text/plain' }, 'Access denied')
    );
    return;
  }

  // Forward flag headers to origin for server-side logging
  request.setHeader('X-Subscriber-Flagged',
    checkResponse.getHeader('X-Subscriber-Flagged')[0]);
  request.setHeader('X-Subscriber-Conditions',
    checkResponse.getHeader('X-Subscriber-Conditions')[0]);
}
```

### AWS CloudFront (Lambda@Edge)

```javascript
const https = require('https');

exports.handler = async (event) => {
  const request = event.Records[0].cf.request;
  const clientIp = request.clientIp;
  const headers = request.headers;

  const subscriberId = (headers['x-subscriber-id'] || [{}])[0].value || 'anonymous';
  const userAgent = (headers['user-agent'] || [{}])[0].value || '';
  const host = (headers['host'] || [{}])[0].value || '';

  const checkResult = await postCheck({
    subscriberId,
    clientIp,
    contentName: request.uri,
    method: request.method,
    path: request.uri,
    host,
    userAgent,
    country: (headers['cloudfront-viewer-country'] || [{}])[0].value || '',
  });

  if (checkResult.action === 'block') {
    return {
      status: '403',
      statusDescription: 'Forbidden',
      body: 'Access denied',
    };
  }

  // Pass flag headers to origin
  request.headers['x-subscriber-flagged'] = [{ value: String(checkResult.flagged) }];
  request.headers['x-subscriber-conditions'] = [{ value: (checkResult.conditions || []).join(',') }];
  return request;
};
```

---

## Authentication

app-rate-limiter uses yeti's built-in auth system. In development mode, all endpoints are accessible without authentication. In production:

- **RequestLog** allows public `read` and `subscribe` access (configured via `@export(public: [read, subscribe])` in the schema). Monitoring dashboards can consume the SSE stream without credentials.
- **RateLimitConfig** requires authentication for all operations. Only authorized users can change thresholds or enforcement actions.
- **POST /check** requires authentication in production. CDN edge workers authenticate via a service account Bearer token or Basic Auth credentials.
- **JWT** and **Basic Auth** are supported (configured in the app's config.yaml `auth:` section).
- For multi-CDN deployments, use yeti-auth's role system to scope different CDN origins to different rate-limiting configurations.

---

## Comparison

| | app-rate-limiter | Redis-based rate limiting | Cloud CDN rate limiting | Custom middleware |
|---|---|---|---|---|
| **Deployment** | Loads with yeti, zero config | Separate Redis cluster + application code | Vendor lock-in, per-provider config | Custom service to build and deploy |
| **Latency** | Single POST, in-process evaluation | Network hop to Redis on every request | Varies by provider, opaque | Depends on implementation |
| **Conditions** | 4 built-in (IP, session, content, volume) | Build your own logic | Usually request-count only | Build your own logic |
| **Configuration** | REST API, runtime changes, no restarts | Application redeploy or config push | Vendor console or API | Application redeploy |
| **Real-time monitoring** | Native SSE + MQTT from schema | Custom pub/sub wiring | Vendor dashboards (delayed) | Custom implementation |
| **Edge integration** | Response headers for any CDN | Custom header logic | Native but vendor-specific | Custom header logic |
| **Auto-expiry** | Built-in 24h TTL on RocksDB | Redis TTL (manual setup) | Provider-managed | Manual cleanup |
| **Auth** | Built-in JWT/Basic/OAuth | Separate auth layer | Provider IAM | Separate auth layer |
| **Binary** | Compiles to native Rust plugin | Redis + application runtime | N/A (SaaS) | Your runtime of choice |
| **Cost** | Self-hosted, zero per-request cost | Redis hosting + compute | Per-request pricing | Compute cost |

---

Built with [Yeti](https://yetirocks.com) | The Performance Platform for Agent-Driven Development
