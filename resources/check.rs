use yeti_sdk::prelude::*;

// Log a request and evaluate rate-limiting conditions in a single call.
//
// POST /app-rate-limiter/check
//   Body: { "subscriberId": "user-123", "clientIp": "1.2.3.4",
//           "sessionId": "sess-abc", "contentName": "video-456",
//           "method": "GET", "path": "/stream/seg-42.ts", ... }
//
// Response headers (for CDN edge integration):
//   X-Subscriber-Flagged: True/False
//   X-Subscriber-Conditions: high_requests,high_ip_count,...
//   X-Subscriber-Action: flag/block/monitor
//
// Conditions evaluated (all within sliding window):
//   1. high_requests    — >maxRequests per subscriber+content (default 50)
//   2. high_ip_count    — >maxIpCount unique IPs per subscriber+content (default 4)
//   3. multi_content    — >maxContentViews unique content per subscriber (default 4)
//   4. multi_session    — >maxSessions unique sessions per subscriber+IP (default 1)
resource!(Check {
    name = "check",
    post(request, ctx) => {
        let body: Value = request.json()?;

        let subscriber_id = body["subscriberId"].as_str()
            .ok_or_else(|| YetiError::Validation("missing subscriberId".into()))?;
        let client_ip = body["clientIp"].as_str()
            .ok_or_else(|| YetiError::Validation("missing clientIp".into()))?;

        let session_id = body["sessionId"].as_str().unwrap_or("");
        let content_name = body["contentName"].as_str().unwrap_or("");
        let now = unix_timestamp()?;
        let now_str = now.to_string();

        let log_table = ctx.get_table("RequestLog")?;
        let config_table = ctx.get_table("RateLimitConfig")?;

        // Load config
        let config = config_table.get("default").await?.unwrap_or(json!({}));
        let window_secs = config["windowSeconds"].as_u64().unwrap_or(10);
        let max_requests = config["maxRequests"].as_u64().unwrap_or(50) as usize;
        let max_ip_count = config["maxIpCount"].as_u64().unwrap_or(4) as usize;
        let max_content = config["maxContentViews"].as_u64().unwrap_or(4) as usize;
        let max_sessions = config["maxSessions"].as_u64().unwrap_or(1) as usize;
        let action = config["action"].as_str().unwrap_or("flag");

        // Write the request log (concurrent with check in spirit — write first)
        let log_id = format!("{}:{}", subscriber_id, now);
        let log_record = json!({
            "id": log_id,
            "subscriberId": subscriber_id,
            "sessionId": session_id,
            "contentName": content_name,
            "clientIp": client_ip,
            "edgeIp": body["edgeIp"].as_str().unwrap_or(""),
            "timestamp": now_str,
            "method": body["method"].as_str().unwrap_or("GET"),
            "path": body["path"].as_str().unwrap_or(""),
            "host": body["host"].as_str().unwrap_or(""),
            "userAgent": body["userAgent"].as_str().unwrap_or(""),
            "country": body["country"].as_str().unwrap_or(""),
            "metadata": body["metadata"].as_str().unwrap_or("{}"),
        });
        log_table.put(&log_id, log_record).await?;

        // Query recent logs for this subscriber within the window
        let window_start = now.saturating_sub(window_secs);
        let all_logs: Vec<Value> = log_table.get_all().await?;
        let window_logs: Vec<&Value> = all_logs.iter().filter(|r| {
            r["subscriberId"].as_str() == Some(subscriber_id)
                && r["timestamp"].as_str()
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(0) >= window_start
                && r["id"].as_str() != Some(&log_id) // exclude current
        }).collect();

        // Evaluate conditions
        let mut conditions: Vec<&str> = Vec::new();

        // 1. High request count per subscriber+content
        if !content_name.is_empty() {
            let content_requests = window_logs.iter()
                .filter(|r| r["contentName"].as_str() == Some(content_name))
                .count();
            if content_requests > max_requests {
                conditions.push("high_requests");
            }
        }

        // 2. High IP count per subscriber+content
        if !content_name.is_empty() {
            let mut ips = std::collections::HashSet::new();
            for r in window_logs.iter().filter(|r| r["contentName"].as_str() == Some(content_name)) {
                if let Some(ip) = r["clientIp"].as_str() {
                    ips.insert(ip);
                }
            }
            ips.insert(client_ip); // include current
            if ips.len() > max_ip_count {
                conditions.push("high_ip_count");
            }
        }

        // 3. Multiple content views per subscriber
        {
            let mut contents = std::collections::HashSet::new();
            for r in &window_logs {
                if let Some(cn) = r["contentName"].as_str() {
                    if !cn.is_empty() { contents.insert(cn); }
                }
            }
            if !content_name.is_empty() { contents.insert(content_name); }
            if contents.len() > max_content {
                conditions.push("multi_content");
            }
        }

        // 4. Multiple sessions per subscriber+IP
        if !session_id.is_empty() {
            let mut sessions = std::collections::HashSet::new();
            for r in window_logs.iter().filter(|r| r["clientIp"].as_str() == Some(client_ip)) {
                if let Some(sid) = r["sessionId"].as_str() {
                    if !sid.is_empty() { sessions.insert(sid); }
                }
            }
            sessions.insert(session_id);
            if sessions.len() > max_sessions {
                conditions.push("multi_session");
            }
        }

        let flagged = !conditions.is_empty();
        let conditions_str = conditions.join(",");

        reply()
            .header("x-subscriber-flagged", if flagged { "True" } else { "False" })
            .header("x-subscriber-conditions", &conditions_str)
            .header("x-subscriber-action", if flagged { action } else { "allow" })
            .code(200)
            .json(json!({
                "subscriberId": subscriber_id,
                "flagged": flagged,
                "conditions": conditions,
                "action": if flagged { action } else { "allow" },
                "windowLogs": window_logs.len(),
                "timestamp": now_str
            }))
    }
});
