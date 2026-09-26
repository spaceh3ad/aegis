# TICKET-011: axum transparent proxy pattern

**Type:** `wayfinder:research`
**Status:** closed

## Resolution

Buffer body as `Bytes`, inspect JSON method, reject or forward via `reqwest`:

```rust
async fn proxy_handler(
    State(state): State<AppState>,
    request: Request,
) -> impl IntoResponse {
    let (parts, body) = request.into_parts();
    let bytes = axum::body::to_bytes(body, usize::MAX).await?;

    if let Ok(rpc) = serde_json::from_slice::<Value>(&bytes) {
        if rpc["method"] == "sendTransaction" {
            if let Some(rejection) = state.pipeline.inspect(&rpc).await {
                return Json(rejection).into_response();
            }
        }
    }

    // Forward raw bytes downstream
    let resp = state.client.post(&state.downstream_url)
        .body(bytes)
        .send().await?;
    (resp.status(), resp.bytes().await?).into_response()
}
```

**Gotchas:**
- Default 2MiB body limit via `DefaultBodyLimit` — override for large transactions
- Pair with `reqwest::Client` (pooled) stored in `State`, not created per-request
