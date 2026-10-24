//! The GitHub API layer: a small trait (so tests can fake it) and the real HTTP client.

use std::fmt;
use std::io::Read;
use std::time::Duration;

use base64::Engine;
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiError {
    /// Could not reach GitHub (DNS, TLS, connection, timeout).
    Network(String),
    /// 401, or 403 that is not a rate limit.
    Unauthorized,
    NotFound,
    RateLimited,
    /// The repository tree is too large for one recursive listing.
    Truncated,
    Other(u16, String),
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApiError::Network(m) => write!(f, "network error: {m}"),
            ApiError::Unauthorized => write!(f, "unauthorized (check the token and its access)"),
            ApiError::NotFound => write!(f, "not found"),
            ApiError::RateLimited => write!(f, "GitHub rate limit reached"),
            ApiError::Truncated => write!(f, "repository tree too large (truncated listing)"),
            ApiError::Other(s, m) => write!(f, "GitHub error {s}: {m}"),
        }
    }
}

impl std::error::Error for ApiError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateError {
    NotFastForward,
    Other(ApiError),
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UpdateError::NotFastForward => write!(f, "branch is not a fast forward"),
            UpdateError::Other(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for UpdateError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeEntry {
    /// Full path in the repository.
    pub path: String,
    pub sha: String,
    pub mode: String,
    /// "blob", "tree" or "commit".
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewEntry {
    /// Full path in the repository.
    pub path: String,
    /// `None` deletes the path.
    pub sha: Option<String>,
    pub mode: String,
}

/// The few Git Data API calls the sync needs. `repo` is `owner/name`.
pub trait GitHubApi {
    /// Commit sha at the head of `branch`.
    fn branch_head(&self, repo: &str, branch: &str) -> Result<String, ApiError>;
    /// Tree sha of a commit.
    fn commit_tree(&self, repo: &str, commit: &str) -> Result<String, ApiError>;
    /// Whole tree, recursively. Must return `ApiError::Truncated` if GitHub truncated it.
    fn tree_recursive(&self, repo: &str, tree: &str) -> Result<Vec<TreeEntry>, ApiError>;
    fn blob(&self, repo: &str, sha: &str) -> Result<Vec<u8>, ApiError>;
    fn create_blob(&self, repo: &str, bytes: &[u8]) -> Result<String, ApiError>;
    fn create_tree(
        &self,
        repo: &str,
        base_tree: &str,
        entries: Vec<NewEntry>,
    ) -> Result<String, ApiError>;
    fn create_commit(
        &self,
        repo: &str,
        message: &str,
        tree: &str,
        parent: &str,
    ) -> Result<String, ApiError>;
    /// Moves the branch to `commit` without force.
    fn update_ref(&self, repo: &str, branch: &str, commit: &str) -> Result<(), UpdateError>;
}

/// Real GitHub over HTTPS (ureq, rustls, webpki-roots). The token is never logged.
pub struct HttpApi {
    token: String,
    base_url: String,
    agent: ureq::Agent,
}

impl fmt::Debug for HttpApi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpApi")
            .field("token", &"<redacted>")
            .field("base_url", &self.base_url)
            .finish()
    }
}

const MAX_BODY: u64 = 256 * 1024 * 1024;

impl HttpApi {
    pub fn new(token: impl Into<String>) -> Self {
        Self::with_base_url(token, "https://api.github.com")
    }

    /// For tests against a local server.
    pub fn with_base_url(token: impl Into<String>, base_url: impl Into<String>) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(15))
            .timeout(Duration::from_secs(120))
            .user_agent("petaricon")
            .build();
        HttpApi {
            token: token.into().trim().to_string(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            agent,
        }
    }

    fn request(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value, ApiError> {
        let url = format!("{}{}", self.base_url, path);
        let req = self
            .agent
            .request(method, &url)
            .set("Authorization", &format!("Bearer {}", self.token))
            .set("Accept", "application/vnd.github+json")
            .set("X-GitHub-Api-Version", "2022-11-28");
        let result = match body {
            Some(b) => req.send_json(b),
            None => req.call(),
        };
        match result {
            Ok(resp) => {
                let mut buf = Vec::new();
                resp.into_reader()
                    .take(MAX_BODY)
                    .read_to_end(&mut buf)
                    .map_err(|e| ApiError::Network(e.to_string()))?;
                serde_json::from_slice(&buf)
                    .map_err(|e| ApiError::Other(200, format!("bad response: {e}")))
            }
            Err(ureq::Error::Status(code, resp)) => {
                let remaining = resp.header("x-ratelimit-remaining").map(str::to_string);
                let text = resp.into_string().unwrap_or_default();
                let message = serde_json::from_str::<Value>(&text)
                    .ok()
                    .and_then(|v| v.get("message").and_then(Value::as_str).map(str::to_string))
                    .unwrap_or(text);
                Err(classify(code, remaining.as_deref(), message))
            }
            // Transport errors can carry the URL but never the headers.
            Err(ureq::Error::Transport(t)) => Err(ApiError::Network(t.to_string())),
        }
    }

    fn get(&self, path: &str) -> Result<Value, ApiError> {
        self.request("GET", path, None)
    }
}

fn classify(code: u16, ratelimit_remaining: Option<&str>, message: String) -> ApiError {
    let lower = message.to_lowercase();
    match code {
        401 => ApiError::Unauthorized,
        404 => ApiError::NotFound,
        429 => ApiError::RateLimited,
        403 if ratelimit_remaining == Some("0") || lower.contains("rate limit") => {
            ApiError::RateLimited
        }
        403 => ApiError::Unauthorized,
        _ => ApiError::Other(code, message),
    }
}

/// Percent-encodes everything but unreserved characters and `/`.
fn enc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b'/') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn str_at<'a>(v: &'a Value, keys: &[&str]) -> Result<&'a str, ApiError> {
    let mut cur = v;
    for k in keys {
        cur = cur
            .get(k)
            .ok_or_else(|| ApiError::Other(200, format!("missing field {}", keys.join("."))))?;
    }
    cur.as_str()
        .ok_or_else(|| ApiError::Other(200, format!("bad field {}", keys.join("."))))
}

impl GitHubApi for HttpApi {
    fn branch_head(&self, repo: &str, branch: &str) -> Result<String, ApiError> {
        let v = self.get(&format!(
            "/repos/{}/git/ref/heads/{}",
            enc(repo),
            enc(branch)
        ))?;
        Ok(str_at(&v, &["object", "sha"])?.to_string())
    }

    fn commit_tree(&self, repo: &str, commit: &str) -> Result<String, ApiError> {
        let v = self.get(&format!("/repos/{}/git/commits/{}", enc(repo), enc(commit)))?;
        Ok(str_at(&v, &["tree", "sha"])?.to_string())
    }

    fn tree_recursive(&self, repo: &str, tree: &str) -> Result<Vec<TreeEntry>, ApiError> {
        let v = self.get(&format!(
            "/repos/{}/git/trees/{}?recursive=1",
            enc(repo),
            enc(tree)
        ))?;
        if v.get("truncated").and_then(Value::as_bool) == Some(true) {
            return Err(ApiError::Truncated);
        }
        let items = v
            .get("tree")
            .and_then(Value::as_array)
            .ok_or_else(|| ApiError::Other(200, "missing field tree".into()))?;
        items
            .iter()
            .map(|e| {
                Ok(TreeEntry {
                    path: str_at(e, &["path"])?.to_string(),
                    sha: str_at(e, &["sha"])?.to_string(),
                    mode: str_at(e, &["mode"])?.to_string(),
                    kind: str_at(e, &["type"])?.to_string(),
                })
            })
            .collect()
    }

    fn blob(&self, repo: &str, sha: &str) -> Result<Vec<u8>, ApiError> {
        let v = self.get(&format!("/repos/{}/git/blobs/{}", enc(repo), enc(sha)))?;
        let content = str_at(&v, &["content"])?;
        let encoding = str_at(&v, &["encoding"])?;
        if encoding != "base64" {
            return Err(ApiError::Other(
                200,
                format!("unknown blob encoding {encoding}"),
            ));
        }
        let compact: String = content.chars().filter(|c| !c.is_whitespace()).collect();
        base64::engine::general_purpose::STANDARD
            .decode(compact)
            .map_err(|e| ApiError::Other(200, format!("bad blob data: {e}")))
    }

    fn create_blob(&self, repo: &str, bytes: &[u8]) -> Result<String, ApiError> {
        let body = json!({
            "content": base64::engine::general_purpose::STANDARD.encode(bytes),
            "encoding": "base64",
        });
        let v = self.request(
            "POST",
            &format!("/repos/{}/git/blobs", enc(repo)),
            Some(body),
        )?;
        Ok(str_at(&v, &["sha"])?.to_string())
    }

    fn create_tree(
        &self,
        repo: &str,
        base_tree: &str,
        entries: Vec<NewEntry>,
    ) -> Result<String, ApiError> {
        let tree: Vec<Value> = entries
            .into_iter()
            .map(|e| json!({"path": e.path, "mode": e.mode, "type": "blob", "sha": e.sha}))
            .collect();
        let body = json!({"base_tree": base_tree, "tree": tree});
        let v = self.request(
            "POST",
            &format!("/repos/{}/git/trees", enc(repo)),
            Some(body),
        )?;
        Ok(str_at(&v, &["sha"])?.to_string())
    }

    fn create_commit(
        &self,
        repo: &str,
        message: &str,
        tree: &str,
        parent: &str,
    ) -> Result<String, ApiError> {
        let body = json!({"message": message, "tree": tree, "parents": [parent]});
        let v = self.request(
            "POST",
            &format!("/repos/{}/git/commits", enc(repo)),
            Some(body),
        )?;
        Ok(str_at(&v, &["sha"])?.to_string())
    }

    fn update_ref(&self, repo: &str, branch: &str, commit: &str) -> Result<(), UpdateError> {
        let body = json!({"sha": commit, "force": false});
        match self.request(
            "PATCH",
            &format!("/repos/{}/git/refs/heads/{}", enc(repo), enc(branch)),
            Some(body),
        ) {
            Ok(_) => Ok(()),
            Err(ApiError::Other(422, m)) if m.to_lowercase().contains("fast forward") => {
                Err(UpdateError::NotFastForward)
            }
            Err(e) => Err(UpdateError::Other(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_statuses() {
        assert_eq!(classify(401, None, "x".into()), ApiError::Unauthorized);
        assert_eq!(
            classify(403, None, "forbidden".into()),
            ApiError::Unauthorized
        );
        assert_eq!(classify(403, Some("0"), "x".into()), ApiError::RateLimited);
        assert_eq!(
            classify(403, None, "You have exceeded a secondary rate limit".into()),
            ApiError::RateLimited
        );
        assert_eq!(classify(429, None, "x".into()), ApiError::RateLimited);
        assert_eq!(classify(404, None, "x".into()), ApiError::NotFound);
        assert_eq!(
            classify(500, None, "boom".into()),
            ApiError::Other(500, "boom".into())
        );
    }

    #[test]
    fn encodes_paths() {
        assert_eq!(enc("feature/a b#1"), "feature/a%20b%231");
    }

    #[test]
    fn debug_redacts_token() {
        let api = HttpApi::new("ghp_secret123");
        let s = format!("{api:?}");
        assert!(!s.contains("secret123"));
        assert!(s.contains("redacted"));
    }
}
