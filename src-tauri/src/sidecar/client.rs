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
            .map_err(SidecarError::Http)?;
        if !resp.status().is_success() {
            return Err(SidecarError::HttpStatus {
                status: resp.status().as_u16(),
            });
        }
        Ok(())
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
            .map_err(SidecarError::Http)?;
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
