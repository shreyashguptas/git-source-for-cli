//! Hand-rolled minimal HTTP/1.1 client for talking to a local Ollama instance.
//!
//! We avoid pulling in `reqwest` (and its ~30 transitive deps) because the
//! Ollama API is plain JSON-over-HTTP on localhost — no TLS, no auth, no
//! redirects, no compression. The tradeoff is ~150 lines of socket wrangling,
//! but in exchange we can stream tokens to the UI as they arrive (the part
//! that makes Cursor's "generate commit message" button feel magical).

use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::timeout;

/// Default connection timeout — Ollama not running should fail fast at startup.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

/// Parse an Ollama base URL (`http://host:port`) into `(host, port)`.
fn parse_base_url(base_url: &str) -> Result<(String, u16)> {
    let s = base_url.trim().trim_end_matches('/');
    let s = s
        .strip_prefix("http://")
        .or_else(|| s.strip_prefix("https://"))
        .unwrap_or(s);
    let mut parts = s.splitn(2, ':');
    let host = parts.next().unwrap_or("localhost").to_string();
    let port: u16 = parts.next().unwrap_or("11434").parse().unwrap_or(11434);
    if host.is_empty() {
        return Err(anyhow!("invalid base_url: {base_url}"));
    }
    Ok((host, port))
}

/// Open a TCP connection to the Ollama instance with a short timeout so
/// "ollama not running" surfaces quickly instead of hanging the UI.
async fn connect(host: &str, port: u16) -> Result<TcpStream> {
    let fut = TcpStream::connect((host, port));
    timeout(CONNECT_TIMEOUT, fut)
        .await
        .with_context(|| format!("timed out connecting to ollama at {host}:{port}"))?
        .with_context(|| format!("connecting to ollama at {host}:{port}"))
}

/// `GET /api/tags` → JSON listing all installed models. Returns sorted names.
pub async fn list_models(base_url: &str) -> Result<Vec<String>> {
    let body = http_get(base_url, "/api/tags").await?;
    let v: serde_json::Value =
        serde_json::from_str(&body).context("parsing /api/tags response")?;
    let arr = v
        .get("models")
        .and_then(|m| m.as_array())
        .context("ollama response missing 'models' field")?;
    let mut names: Vec<String> = arr
        .iter()
        .filter_map(|m| m.get("name").and_then(|n| n.as_str()).map(String::from))
        .collect();
    names.sort();
    names.dedup();
    Ok(names)
}

/// Stream a generation. Tokens are sent through `tokens` as they arrive.
/// Returns the full assembled response on completion. Drops `tokens` on
/// completion or error so the receiver can finalize.
pub async fn generate_stream(
    base_url: &str,
    model: &str,
    system: &str,
    prompt: &str,
    tokens: mpsc::Sender<String>,
) -> Result<String> {
    let (host, port) = parse_base_url(base_url)?;
    let mut stream = connect(&host, port).await?;

    let body = serde_json::to_string(&serde_json::json!({
        "model": model,
        "system": system,
        "prompt": prompt,
        "stream": true,
        // Disable thinking on models that support it (Gemma 4, Qwen 3, etc.).
        // When `think` defaults to true, the model spends its num_predict
        // budget emitting reasoning into a separate `thinking` field, leaving
        // `response` empty — the modal then shows "no text". For a one-line
        // commit subject we want the answer directly. Older models that don't
        // support `think` ignore the field, so this is safe everywhere.
        "think": false,
        // No length cap on the model's output. `num_predict: -1` lifts the
        // predict-count cap, and `num_ctx: 32768` lifts Ollama's default 4K
        // context window so the model has room for the diff, the recent-
        // subject style block, the system prompt, AND a multi-paragraph
        // body. Without num_ctx, long inputs leave so little room that the
        // model stops mid-sentence and the commit message looks truncated.
        // 32K is supported by every modern small model (Gemma 4, Qwen 3,
        // Llama 3.2, etc.) and Ollama silently clamps to the model's max
        // when smaller, so this is safe across the board.
        "options": {
            "num_predict": -1,
            "num_ctx": 32768,
            "temperature": 0.2,
        },
        // Unload the model from memory immediately after this request
        // finishes. Without this, Ollama keeps the model resident for ~5
        // minutes; for a one-shot commit message that's wasted RAM. The
        // tradeoff is a cold start on the next generation, but the user has
        // explicitly asked us to free the cache.
        "keep_alive": 0,
    }))
    .context("building generate request body")?;

    let req = format!(
        "POST /api/generate HTTP/1.1\r\n\
         Host: {host}:{port}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        body.len()
    );
    stream.write_all(req.as_bytes()).await?;
    stream.flush().await?;

    let mut reader = BufReader::new(stream);
    let (status, chunked, _len) = read_headers(&mut reader).await?;
    if status != 200 {
        // Read whatever body is there for a useful error message.
        let mut tail = String::new();
        let _ = reader.read_to_string(&mut tail).await;
        return Err(anyhow!(
            "ollama returned HTTP {status}: {}",
            tail.trim().chars().take(200).collect::<String>()
        ));
    }

    let mut full = String::new();

    if chunked {
        // Each HTTP chunk may contain one or more newline-delimited JSON
        // objects (Ollama's streaming format). We accumulate raw bytes across
        // chunks because a JSON line can theoretically span chunk boundaries.
        let mut leftover = String::new();
        loop {
            let mut size_line = String::new();
            reader.read_line(&mut size_line).await?;
            let size_str = size_line.trim();
            if size_str.is_empty() {
                continue;
            }
            // Strip optional chunk-extension after `;`
            let size_str = size_str.split(';').next().unwrap_or(size_str);
            let size = usize::from_str_radix(size_str, 16)
                .with_context(|| format!("invalid chunk size: {size_str:?}"))?;
            if size == 0 {
                // Final chunk — read the trailing CRLF (or trailers) and exit.
                let mut t = String::new();
                let _ = reader.read_line(&mut t).await;
                break;
            }
            let mut buf = vec![0u8; size];
            reader.read_exact(&mut buf).await?;
            // Trailing CRLF after the chunk body.
            let mut crlf = [0u8; 2];
            reader.read_exact(&mut crlf).await?;

            leftover.push_str(&String::from_utf8_lossy(&buf));
            while let Some(nl) = leftover.find('\n') {
                let line = leftover[..nl].to_string();
                leftover.drain(..=nl);
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let v: serde_json::Value = match serde_json::from_str(line) {
                    Ok(v) => v,
                    Err(_) => continue, // skip malformed lines defensively
                };
                if let Some(token) = v.get("response").and_then(|s| s.as_str()) {
                    if !token.is_empty() {
                        full.push_str(token);
                        if tokens.send(token.to_string()).await.is_err() {
                            // Receiver gone — caller cancelled.
                            return Ok(full);
                        }
                    }
                }
                if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
                    return Err(anyhow!("ollama: {err}"));
                }
                if v.get("done").and_then(|b| b.as_bool()) == Some(true) {
                    return Ok(full);
                }
            }
        }
    } else {
        // Non-chunked: a single JSON document.
        let mut body = String::new();
        reader.read_to_string(&mut body).await?;
        let v: serde_json::Value = serde_json::from_str(&body)
            .context("parsing non-streaming generate response")?;
        if let Some(text) = v.get("response").and_then(|s| s.as_str()) {
            full = text.to_string();
            let _ = tokens.send(full.clone()).await;
        }
        if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
            return Err(anyhow!("ollama: {err}"));
        }
    }

    Ok(full)
}

/// Plain `GET <path>` returning the response body as a UTF-8 string.
async fn http_get(base_url: &str, path: &str) -> Result<String> {
    let (host, port) = parse_base_url(base_url)?;
    let mut stream = connect(&host, port).await?;
    let req = format!(
        "GET {path} HTTP/1.1\r\n\
         Host: {host}:{port}\r\n\
         Accept: application/json\r\n\
         Connection: close\r\n\
         \r\n"
    );
    stream.write_all(req.as_bytes()).await?;
    stream.flush().await?;

    let mut reader = BufReader::new(stream);
    let (status, chunked, content_length) = read_headers(&mut reader).await?;
    if status != 200 {
        return Err(anyhow!("ollama returned HTTP {status}"));
    }

    if chunked {
        read_chunked_body(&mut reader).await
    } else {
        // Prefer Content-Length when available (server should send it for /api/tags).
        let mut body = String::new();
        if let Some(n) = content_length {
            let mut buf = vec![0u8; n];
            reader.read_exact(&mut buf).await?;
            body = String::from_utf8_lossy(&buf).into_owned();
        } else {
            reader.read_to_string(&mut body).await?;
        }
        Ok(body)
    }
}

/// Read HTTP response headers up to the blank `\r\n` separator.
/// Returns (status_code, is_chunked, content_length).
async fn read_headers(
    reader: &mut BufReader<TcpStream>,
) -> Result<(u16, bool, Option<usize>)> {
    let mut status = 0u16;
    let mut chunked = false;
    let mut content_length: Option<usize> = None;
    let mut first = true;
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            break;
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        if first {
            first = false;
            // "HTTP/1.1 200 OK"
            let mut parts = line.split_whitespace();
            parts.next(); // version
            if let Some(code) = parts.next() {
                status = code.parse().unwrap_or(0);
            }
            continue;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("transfer-encoding:") {
            if rest.contains("chunked") {
                chunked = true;
            }
        } else if let Some(rest) = lower.strip_prefix("content-length:") {
            content_length = rest.trim().parse().ok();
        }
    }
    Ok((status, chunked, content_length))
}

/// Read a chunked-transfer-encoded body to completion.
async fn read_chunked_body(reader: &mut BufReader<TcpStream>) -> Result<String> {
    let mut full = Vec::new();
    loop {
        let mut size_line = String::new();
        reader.read_line(&mut size_line).await?;
        let size_str = size_line.trim();
        if size_str.is_empty() {
            continue;
        }
        let size_str = size_str.split(';').next().unwrap_or(size_str);
        let size = usize::from_str_radix(size_str, 16)
            .with_context(|| format!("invalid chunk size: {size_str:?}"))?;
        if size == 0 {
            let mut t = String::new();
            let _ = reader.read_line(&mut t).await;
            break;
        }
        let mut buf = vec![0u8; size];
        reader.read_exact(&mut buf).await?;
        full.extend_from_slice(&buf);
        let mut crlf = [0u8; 2];
        reader.read_exact(&mut crlf).await?;
    }
    Ok(String::from_utf8_lossy(&full).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_base_url() {
        assert_eq!(
            parse_base_url("http://localhost:11434").unwrap(),
            ("localhost".to_string(), 11434)
        );
        assert_eq!(
            parse_base_url("http://127.0.0.1:11434/").unwrap(),
            ("127.0.0.1".to_string(), 11434)
        );
        assert_eq!(
            parse_base_url("localhost:8080").unwrap(),
            ("localhost".to_string(), 8080)
        );
        // Default port when missing.
        assert_eq!(
            parse_base_url("http://localhost").unwrap(),
            ("localhost".to_string(), 11434)
        );
    }
}
