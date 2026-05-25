use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_LENGTH, RANGE};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;

use crate::error::{SidecarError, SidecarResult};

/// What we know about an asset before fetching it.
#[derive(Clone, Debug)]
pub struct DownloadSpec {
    pub url: String,
    /// Final destination on disk.
    pub destination: PathBuf,
    /// Optional expected SHA-256 (lowercase hex). If `None`, the checksum is
    /// computed and returned but not enforced.
    pub expected_sha256: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DownloadProgress {
    pub url: String,
    pub bytes_downloaded: u64,
    pub bytes_total: Option<u64>,
}

pub type DownloadSink = Arc<dyn Fn(&DownloadProgress) + Send + Sync>;

/// Resumable HTTP download via Range requests. Writes to `<dest>.part`, then
/// renames atomically on success. Computes SHA-256 on the fly.
pub async fn download_resumable(
    http: &reqwest::Client,
    spec: &DownloadSpec,
    on_progress: DownloadSink,
) -> SidecarResult<String> {
    if let Some(parent) = spec.destination.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(SidecarError::Io)?;
    }

    // Short-circuit: if destination already exists and matches the expected
    // checksum, skip the download entirely.
    if spec.destination.exists() {
        if let Some(expected) = &spec.expected_sha256 {
            let got = sha256_file(&spec.destination).await?;
            if &got == expected {
                tracing::info!(path = %spec.destination.display(), "asset already present, skipping download");
                return Ok(got);
            }
        }
    }

    let part_path = part_path(&spec.destination);
    let mut existing = 0u64;
    if let Ok(meta) = tokio::fs::metadata(&part_path).await {
        existing = meta.len();
    }

    let mut headers = HeaderMap::new();
    if existing > 0 {
        let range = format!("bytes={existing}-");
        if let Ok(v) = HeaderValue::from_str(&range) {
            headers.insert(RANGE, v);
        }
    }

    let resp = http
        .get(&spec.url)
        .headers(headers)
        .send()
        .await
        .map_err(SidecarError::Http)?;

    let status = resp.status();
    if !(status.is_success() || status.as_u16() == 206) {
        // 416 means our partial is already complete; verify and finish.
        if status.as_u16() == 416 && spec.destination.exists() {
            return verify_and_finish(&spec.destination, spec.expected_sha256.as_deref()).await;
        }
        return Err(SidecarError::HttpStatus {
            status: status.as_u16(),
        });
    }

    let total = total_size(resp.headers(), existing, status.as_u16());

    // Compose the running hasher from the existing .part bytes (so resume
    // produces the same checksum as a fresh download).
    let mut hasher = Sha256::new();
    let mut downloaded = 0u64;
    if existing > 0 {
        hasher = sha256_running(&part_path).await?;
        downloaded = existing;
        emit(&on_progress, &spec.url, downloaded, total);
    }

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&part_path)
        .await
        .map_err(SidecarError::Io)?;

    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(SidecarError::Http)?;
        hasher.update(&chunk);
        file.write_all(&chunk).await.map_err(SidecarError::Io)?;
        downloaded += chunk.len() as u64;
        emit(&on_progress, &spec.url, downloaded, total);
    }
    file.flush().await.map_err(SidecarError::Io)?;
    drop(file);

    let actual = hex::encode(hasher.finalize());
    if let Some(expected) = &spec.expected_sha256 {
        if &actual != expected {
            // Leave the .part in place so a subsequent run can either resume
            // (if the partial is still aligned) or be deleted by the caller.
            return Err(SidecarError::ChecksumMismatch {
                expected: expected.clone(),
                actual,
            });
        }
    }

    tokio::fs::rename(&part_path, &spec.destination)
        .await
        .map_err(SidecarError::Io)?;
    Ok(actual)
}

async fn verify_and_finish(dest: &Path, expected: Option<&str>) -> SidecarResult<String> {
    let got = sha256_file(dest).await?;
    if let Some(exp) = expected {
        if got != exp {
            return Err(SidecarError::ChecksumMismatch {
                expected: exp.to_owned(),
                actual: got,
            });
        }
    }
    Ok(got)
}

fn total_size(headers: &HeaderMap, existing: u64, status: u16) -> Option<u64> {
    if status == 206 {
        // Content-Range: bytes start-end/total
        if let Some(cr) = headers.get("content-range") {
            if let Ok(s) = cr.to_str() {
                if let Some(slash) = s.find('/') {
                    if let Ok(n) = s[slash + 1..].parse::<u64>() {
                        return Some(n);
                    }
                }
            }
        }
    }
    if let Some(cl) = headers.get(CONTENT_LENGTH) {
        if let Ok(s) = cl.to_str() {
            if let Ok(n) = s.parse::<u64>() {
                return Some(existing + n);
            }
        }
    }
    None
}

fn emit(sink: &DownloadSink, url: &str, downloaded: u64, total: Option<u64>) {
    sink(&DownloadProgress {
        url: url.to_owned(),
        bytes_downloaded: downloaded,
        bytes_total: total,
    });
}

fn part_path(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

async fn sha256_file(path: &Path) -> SidecarResult<String> {
    let hasher = sha256_running(path).await?;
    Ok(hex::encode(hasher.finalize()))
}

async fn sha256_running(path: &Path) -> SidecarResult<Sha256> {
    use tokio::io::AsyncReadExt;
    let mut f = tokio::fs::File::open(path)
        .await
        .map_err(SidecarError::Io)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf).await.map_err(SidecarError::Io)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tempfile::TempDir;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn sink() -> (DownloadSink, Arc<Mutex<Vec<DownloadProgress>>>) {
        let log: Arc<Mutex<Vec<DownloadProgress>>> = Arc::new(Mutex::new(Vec::new()));
        let log2 = log.clone();
        let sink: DownloadSink = Arc::new(move |p| log2.lock().unwrap().push(p.clone()));
        (sink, log)
    }

    fn expected_sha(body: &[u8]) -> String {
        hex::encode(Sha256::digest(body))
    }

    #[tokio::test(flavor = "current_thread")]
    async fn happy_path_downloads_and_verifies() {
        let server = MockServer::start().await;
        let body = vec![7u8; 1024 * 16];
        Mock::given(method("GET"))
            .and(path("/asset.bin"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body.clone()))
            .mount(&server)
            .await;

        let tmp = TempDir::new().unwrap();
        let dest = tmp.path().join("asset.bin");
        let (s, _log) = sink();
        let spec = DownloadSpec {
            url: format!("{}/asset.bin", server.uri()),
            destination: dest.clone(),
            expected_sha256: Some(expected_sha(&body)),
        };

        let actual = download_resumable(&reqwest::Client::new(), &spec, s)
            .await
            .unwrap();
        assert_eq!(actual, expected_sha(&body));
        let on_disk = tokio::fs::read(&dest).await.unwrap();
        assert_eq!(on_disk, body);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn checksum_mismatch_surfaces_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/x.bin"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![1u8; 8]))
            .mount(&server)
            .await;

        let tmp = TempDir::new().unwrap();
        let (s, _) = sink();
        let spec = DownloadSpec {
            url: format!("{}/x.bin", server.uri()),
            destination: tmp.path().join("x.bin"),
            expected_sha256: Some("00".repeat(32)),
        };
        let err = download_resumable(&reqwest::Client::new(), &spec, s)
            .await
            .unwrap_err();
        assert!(matches!(err, SidecarError::ChecksumMismatch { .. }));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn resume_skips_already_downloaded_bytes() {
        let server = MockServer::start().await;
        let body = (0u8..=255).collect::<Vec<u8>>().repeat(64); // 16 KiB deterministic
        let suffix = body[1024..].to_vec();

        // The resumed request MUST include a Range header starting at 1024.
        Mock::given(method("GET"))
            .and(path("/r.bin"))
            .and(header("range", "bytes=1024-"))
            .respond_with(
                ResponseTemplate::new(206)
                    .insert_header(
                        "content-range",
                        format!("bytes 1024-{}/{}", body.len() - 1, body.len()).as_str(),
                    )
                    .set_body_bytes(suffix),
            )
            .mount(&server)
            .await;

        let tmp = TempDir::new().unwrap();
        let dest = tmp.path().join("r.bin");
        let part = tmp.path().join("r.bin.part");
        tokio::fs::write(&part, &body[..1024]).await.unwrap();

        let (s, _) = sink();
        let spec = DownloadSpec {
            url: format!("{}/r.bin", server.uri()),
            destination: dest.clone(),
            expected_sha256: Some(expected_sha(&body)),
        };

        let actual = download_resumable(&reqwest::Client::new(), &spec, s)
            .await
            .unwrap();
        assert_eq!(actual, expected_sha(&body));
        let on_disk = tokio::fs::read(&dest).await.unwrap();
        assert_eq!(on_disk, body);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn existing_file_with_matching_checksum_skips_network() {
        let server = MockServer::start().await;
        // The server should never be hit; if it is, the response will explode
        // the test because no matcher is registered.
        let body = b"hello-world".to_vec();
        let tmp = TempDir::new().unwrap();
        let dest = tmp.path().join("e.bin");
        tokio::fs::write(&dest, &body).await.unwrap();

        let (s, _) = sink();
        let spec = DownloadSpec {
            url: format!("{}/e.bin", server.uri()),
            destination: dest.clone(),
            expected_sha256: Some(expected_sha(&body)),
        };
        let actual = download_resumable(&reqwest::Client::new(), &spec, s)
            .await
            .unwrap();
        assert_eq!(actual, expected_sha(&body));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn progress_callback_is_invoked() {
        let server = MockServer::start().await;
        let body = vec![0u8; 4096];
        Mock::given(method("GET"))
            .and(path("/p.bin"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body.clone()))
            .mount(&server)
            .await;
        let tmp = TempDir::new().unwrap();
        let (s, log) = sink();
        let spec = DownloadSpec {
            url: format!("{}/p.bin", server.uri()),
            destination: tmp.path().join("p.bin"),
            expected_sha256: None,
        };
        download_resumable(&reqwest::Client::new(), &spec, s)
            .await
            .unwrap();
        assert!(!log.lock().unwrap().is_empty());
        let last = log.lock().unwrap().last().cloned().unwrap();
        assert_eq!(last.bytes_downloaded, body.len() as u64);
    }
}
