use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::{SidecarError, SidecarResult};

/// What the embedding pipeline asks for. Implemented by `HttpSidecarClient`
/// against the real sidecar and by `FakeEmbedder` in tests.
#[async_trait]
pub trait EmbeddingClient: Send + Sync {
    async fn embed(&self, text: &str) -> SidecarResult<Vec<f32>>;
}

/// What the classification pipeline asks for. Same trait pattern; a separate
/// trait so the embedding sidecar and classifier sidecar can live as two
/// distinct processes on two ports.
#[async_trait]
pub trait CompletionClient: Send + Sync {
    /// Send a single user prompt + a JSON-schema constraint, get back the
    /// raw model output (which should be parseable JSON matching the schema).
    async fn complete_json(
        &self,
        prompt: &str,
        schema: &serde_json::Value,
    ) -> SidecarResult<String>;
}

/// HTTP client for the llama.cpp `/embedding` (and `/v1/embeddings`) endpoint.
#[derive(Clone)]
pub struct HttpSidecarClient {
    http: reqwest::Client,
    base_url: String,
}

impl HttpSidecarClient {
    pub fn new(http: reqwest::Client, port: u16) -> Self {
        Self {
            http,
            base_url: format!("http://127.0.0.1:{port}"),
        }
    }

    pub fn with_base_url(http: reqwest::Client, base_url: String) -> Self {
        Self { http, base_url }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub async fn health(&self) -> SidecarResult<()> {
        let resp = self
            .http
            .get(format!("{}/health", self.base_url))
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| self.classify_send_error(e))?;
        if !resp.status().is_success() {
            return Err(SidecarError::HttpStatus {
                status: resp.status().as_u16(),
            });
        }
        Ok(())
    }

    /// A `reqwest::Error` from `.send()` could be a timeout, a TLS failure,
    /// or — most commonly when the sidecar isn't running — a TCP connect
    /// failure. Surface the last case as `NotReachable` so the UI can show
    /// a "start llama-server" hint instead of a stacked socket error.
    fn classify_send_error(&self, e: reqwest::Error) -> SidecarError {
        if e.is_connect() {
            SidecarError::NotReachable {
                base_url: self.base_url.clone(),
            }
        } else {
            SidecarError::Http(e)
        }
    }
}

#[derive(Serialize)]
struct EmbedRequest<'a> {
    content: &'a str,
}

/// llama.cpp returns one of two shapes depending on version:
///  - `{ "embedding": [..floats..] }`
///  - `{ "embedding": [[..floats..]] }`  (sequence dimension)
#[derive(Deserialize)]
#[serde(untagged)]
enum EmbedResponse {
    Flat { embedding: Vec<f32> },
    Nested { embedding: Vec<Vec<f32>> },
}

#[derive(Serialize)]
struct CompletionRequest<'a> {
    prompt: &'a str,
    n_predict: i32,
    temperature: f32,
    cache_prompt: bool,
    json_schema: &'a serde_json::Value,
}

#[derive(Deserialize)]
struct CompletionResponse {
    content: String,
}

#[async_trait]
impl CompletionClient for HttpSidecarClient {
    async fn complete_json(
        &self,
        prompt: &str,
        schema: &serde_json::Value,
    ) -> SidecarResult<String> {
        let url = format!("{}/completion", self.base_url);
        let body = CompletionRequest {
            prompt,
            n_predict: 256,
            temperature: 0.0,
            cache_prompt: true,
            json_schema: schema,
        };
        let resp = self
            .http
            .post(&url)
            .json(&body)
            .timeout(Duration::from_secs(120))
            .send()
            .await
            .map_err(|e| self.classify_send_error(e))?;
        if !resp.status().is_success() {
            return Err(SidecarError::HttpStatus {
                status: resp.status().as_u16(),
            });
        }
        let parsed: CompletionResponse = resp.json().await.map_err(SidecarError::Http)?;
        let trimmed = parsed.content.trim().to_owned();
        if trimmed.is_empty() {
            return Err(SidecarError::Request("empty completion".to_owned()));
        }
        Ok(trimmed)
    }
}

#[async_trait]
impl EmbeddingClient for HttpSidecarClient {
    async fn embed(&self, text: &str) -> SidecarResult<Vec<f32>> {
        let url = format!("{}/embedding", self.base_url);
        let resp = self
            .http
            .post(&url)
            .json(&EmbedRequest { content: text })
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| self.classify_send_error(e))?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            return Err(SidecarError::HttpStatus { status });
        }
        let body: serde_json::Value = resp.json().await.map_err(SidecarError::Http)?;
        let parsed: EmbedResponse =
            serde_json::from_value(body).map_err(SidecarError::Malformed)?;
        let vec = match parsed {
            EmbedResponse::Flat { embedding } => embedding,
            EmbedResponse::Nested { embedding } => {
                embedding.into_iter().next().unwrap_or_default()
            }
        };
        if vec.is_empty() {
            return Err(SidecarError::EmptyEmbedding);
        }
        Ok(vec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test(flavor = "current_thread")]
    async fn embed_parses_flat_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/embedding"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "embedding": [0.1f32, 0.2, 0.3]
            })))
            .mount(&server)
            .await;
        let client =
            HttpSidecarClient::with_base_url(reqwest::Client::new(), server.uri().to_string());
        let v = client.embed("hello").await.unwrap();
        assert_eq!(v, vec![0.1, 0.2, 0.3]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn embed_parses_nested_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/embedding"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "embedding": [[0.5f32, 0.25]]
            })))
            .mount(&server)
            .await;
        let client =
            HttpSidecarClient::with_base_url(reqwest::Client::new(), server.uri().to_string());
        let v = client.embed("hi").await.unwrap();
        assert_eq!(v, vec![0.5, 0.25]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn embed_errors_on_http_5xx() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/embedding"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let client =
            HttpSidecarClient::with_base_url(reqwest::Client::new(), server.uri().to_string());
        let err = client.embed("hi").await.unwrap_err();
        assert!(matches!(err, SidecarError::HttpStatus { status: 503 }));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn embed_surfaces_not_reachable_when_nothing_is_listening() {
        // Point at a port we just bound and immediately released so that any
        // connect attempt is reliably refused. Avoids racing a real listener.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let base_url = format!("http://127.0.0.1:{port}");
        let client = HttpSidecarClient::with_base_url(reqwest::Client::new(), base_url.clone());
        let err = client.embed("hi").await.unwrap_err();
        match err {
            SidecarError::NotReachable { base_url: got } => assert_eq!(got, base_url),
            other => panic!("expected NotReachable, got: {other:?}"),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn embed_errors_on_empty_embedding() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/embedding"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "embedding": []
            })))
            .mount(&server)
            .await;
        let client =
            HttpSidecarClient::with_base_url(reqwest::Client::new(), server.uri().to_string());
        let err = client.embed("hi").await.unwrap_err();
        assert!(matches!(err, SidecarError::EmptyEmbedding));
    }
}
