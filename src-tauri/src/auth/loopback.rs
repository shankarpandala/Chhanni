use std::collections::HashMap;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use url::Url;

use crate::error::{AuthError, AuthResult};

/// Result of a successful OAuth redirect.
#[derive(Debug, Clone)]
pub struct CallbackResult {
    pub code: String,
    pub state: String,
}

/// Bound loopback server. Hold this for the duration of the OAuth flow.
pub struct LoopbackServer {
    listener: TcpListener,
    port: u16,
}

impl LoopbackServer {
    pub async fn bind() -> AuthResult<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(AuthError::LoopbackBind)?;
        let port = listener.local_addr().map_err(AuthError::LoopbackBind)?.port();
        Ok(Self { listener, port })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn redirect_uri(&self) -> String {
        format!("http://127.0.0.1:{}/callback", self.port)
    }

    /// Wait for the first matching `/callback` request, parse `code` + `state`,
    /// and write a small HTML confirmation page back to the browser. Times out
    /// after `timeout`.
    pub async fn wait_for_callback(self, timeout: Duration) -> AuthResult<CallbackResult> {
        let timeout_secs = timeout.as_secs();
        tokio::time::timeout(timeout, self.accept_loop())
            .await
            .map_err(|_| AuthError::CallbackTimeout { seconds: timeout_secs })?
    }

    async fn accept_loop(self) -> AuthResult<CallbackResult> {
        loop {
            let (mut stream, _) = self.listener.accept().await.map_err(AuthError::LoopbackIo)?;

            let mut buf = vec![0u8; 4096];
            let n = stream.read(&mut buf).await.map_err(AuthError::LoopbackIo)?;
            if n == 0 {
                continue;
            }
            let request = String::from_utf8_lossy(&buf[..n]);

            let Some(request_line) = request.lines().next() else {
                continue;
            };
            let mut parts = request_line.split_whitespace();
            let method = parts.next().unwrap_or("");
            let target = parts.next().unwrap_or("");

            if method != "GET" || !target.starts_with("/callback") {
                write_response(&mut stream, 404, "Not Found").await;
                continue;
            }

            let params = parse_query(target);

            if let Some(err) = params.get("error") {
                write_response(
                    &mut stream,
                    400,
                    "Authorization failed. You can close this tab.",
                )
                .await;
                return Err(AuthError::ProviderError(err.clone()));
            }

            let Some(code) = params.get("code").cloned() else {
                write_response(&mut stream, 400, "Missing code parameter").await;
                continue;
            };
            let Some(state) = params.get("state").cloned() else {
                write_response(&mut stream, 400, "Missing state parameter").await;
                continue;
            };

            write_response(
                &mut stream,
                200,
                "Chhanni: connection authorized. You can close this tab.",
            )
            .await;
            return Ok(CallbackResult { code, state });
        }
    }
}

fn parse_query(target: &str) -> HashMap<String, String> {
    let dummy = format!("http://127.0.0.1{target}");
    let Ok(url) = Url::parse(&dummy) else {
        return HashMap::new();
    };
    url.query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

async fn write_response(stream: &mut tokio::net::TcpStream, status: u16, body: &str) {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "Error",
    };
    let body_bytes = body.as_bytes();
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\
         Content-Length: {len}\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        len = body_bytes.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_query_extracts_pairs() {
        let q = parse_query("/callback?code=abc&state=xyz");
        assert_eq!(q.get("code").map(String::as_str), Some("abc"));
        assert_eq!(q.get("state").map(String::as_str), Some("xyz"));
    }

    #[test]
    fn parse_query_handles_url_encoding() {
        let q = parse_query("/callback?code=a%2Bb&state=hello%20world");
        assert_eq!(q.get("code").map(String::as_str), Some("a+b"));
        assert_eq!(q.get("state").map(String::as_str), Some("hello world"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn loopback_round_trip() {
        let server = LoopbackServer::bind().await.unwrap();
        let port = server.port();
        assert!(port > 0);

        let client_task = tokio::spawn(async move {
            let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .unwrap();
            let req = "GET /callback?code=abc123&state=stateval HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
            stream.write_all(req.as_bytes()).await.unwrap();
            let mut resp = Vec::new();
            tokio::io::AsyncReadExt::read_to_end(&mut stream, &mut resp)
                .await
                .unwrap();
            String::from_utf8_lossy(&resp).into_owned()
        });

        let cb = server
            .wait_for_callback(Duration::from_secs(5))
            .await
            .unwrap();
        let resp = client_task.await.unwrap();

        assert_eq!(cb.code, "abc123");
        assert_eq!(cb.state, "stateval");
        assert!(resp.contains("200 OK"));
        assert!(resp.contains("authorized"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn loopback_surfaces_provider_error() {
        let server = LoopbackServer::bind().await.unwrap();
        let port = server.port();

        let client_task = tokio::spawn(async move {
            let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .unwrap();
            let req = "GET /callback?error=access_denied HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
            stream.write_all(req.as_bytes()).await.unwrap();
        });

        let err = server
            .wait_for_callback(Duration::from_secs(5))
            .await
            .unwrap_err();
        client_task.await.unwrap();
        match err {
            AuthError::ProviderError(e) => assert_eq!(e, "access_denied"),
            other => panic!("expected ProviderError, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn loopback_times_out_when_no_callback() {
        let server = LoopbackServer::bind().await.unwrap();
        let err = server
            .wait_for_callback(Duration::from_millis(50))
            .await
            .unwrap_err();
        assert!(matches!(err, AuthError::CallbackTimeout { .. }));
    }
}
