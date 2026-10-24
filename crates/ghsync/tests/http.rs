//! HttpApi against a tiny local HTTP server (never the real GitHub).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::thread;

use ghsync::*;

struct Seen {
    request_line: String,
    headers: Vec<(String, String)>,
    body: String,
}

/// Serves each canned (status, extra headers, body) once, in order.
fn serve(responses: Vec<(u16, &'static str, String)>) -> (String, thread::JoinHandle<Vec<Seen>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    let h = thread::spawn(move || {
        let mut seen = Vec::new();
        for (status, extra, body) in responses {
            let (mut s, _) = l.accept().unwrap();
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut line = String::new();
            r.read_line(&mut line).unwrap();
            let mut headers = Vec::new();
            let mut len = 0;
            loop {
                let mut h = String::new();
                r.read_line(&mut h).unwrap();
                if h.trim().is_empty() {
                    break;
                }
                let (k, v) = h.trim().split_once(':').unwrap();
                if k.eq_ignore_ascii_case("content-length") {
                    len = v.trim().parse().unwrap();
                }
                headers.push((k.to_lowercase(), v.trim().to_string()));
            }
            let mut b = vec![0; len];
            r.read_exact(&mut b).unwrap();
            seen.push(Seen {
                request_line: line.trim().into(),
                headers,
                body: String::from_utf8(b).unwrap(),
            });
            write!(
                s,
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
        seen
    });
    (url, h)
}

fn header<'a>(s: &'a Seen, k: &str) -> Option<&'a str> {
    s.headers
        .iter()
        .find(|(n, _)| n == k)
        .map(|(_, v)| v.as_str())
}

#[test]
fn branch_head_sends_required_headers() {
    let (url, h) = serve(vec![(200, "", r#"{"object":{"sha":"abc123"}}"#.into())]);
    let api = HttpApi::with_base_url("tok_secret", url);
    assert_eq!(api.branch_head("o/r", "feat/x").unwrap(), "abc123");
    let seen = h.join().unwrap();
    assert_eq!(
        seen[0].request_line,
        "GET /repos/o/r/git/ref/heads/feat/x HTTP/1.1"
    );
    assert_eq!(header(&seen[0], "authorization"), Some("Bearer tok_secret"));
    assert_eq!(
        header(&seen[0], "accept"),
        Some("application/vnd.github+json")
    );
    assert!(header(&seen[0], "x-github-api-version").is_some());
    assert!(header(&seen[0], "user-agent").is_some());
}

#[test]
fn error_statuses_are_classified() {
    let (url, h) = serve(vec![
        (401, "", r#"{"message":"Bad credentials"}"#.into()),
        (403, "", r#"{"message":"Resource not accessible"}"#.into()),
        (
            403,
            "x-ratelimit-remaining: 0\r\n",
            r#"{"message":"API rate limit exceeded"}"#.into(),
        ),
        (404, "", r#"{"message":"Not Found"}"#.into()),
        (500, "", r#"{"message":"boom"}"#.into()),
    ]);
    let api = HttpApi::with_base_url("t", url);
    assert_eq!(api.branch_head("o/r", "b"), Err(ApiError::Unauthorized));
    assert_eq!(api.branch_head("o/r", "b"), Err(ApiError::Unauthorized));
    assert_eq!(api.branch_head("o/r", "b"), Err(ApiError::RateLimited));
    assert_eq!(api.branch_head("o/r", "b"), Err(ApiError::NotFound));
    assert_eq!(
        api.branch_head("o/r", "b"),
        Err(ApiError::Other(500, "boom".into()))
    );
    h.join().unwrap();
}

#[test]
fn network_failure_does_not_leak_token() {
    let api = HttpApi::with_base_url("tok_secret", "http://127.0.0.1:1");
    let e = api.branch_head("o/r", "b").unwrap_err();
    assert!(matches!(e, ApiError::Network(_)));
    assert!(!format!("{e:?} {e}").contains("tok_secret"));
}

#[test]
fn tree_blob_and_truncation() {
    let tree = r#"{"sha":"t","truncated":false,"tree":[{"path":"a/b.txt","mode":"100644","type":"blob","sha":"s1"},{"path":"a","mode":"040000","type":"tree","sha":"s2"}]}"#;
    let (url, h) = serve(vec![
        (200, "", tree.into()),
        (200, "", r#"{"sha":"t","truncated":true,"tree":[]}"#.into()),
        (
            200,
            "",
            r#"{"content":"aGVs\nbG8K\n","encoding":"base64"}"#.into(),
        ),
    ]);
    let api = HttpApi::with_base_url("t", url);
    let entries = api.tree_recursive("o/r", "t").unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(
        entries[0],
        TreeEntry {
            path: "a/b.txt".into(),
            sha: "s1".into(),
            mode: "100644".into(),
            kind: "blob".into()
        }
    );
    assert_eq!(api.tree_recursive("o/r", "t"), Err(ApiError::Truncated));
    assert_eq!(api.blob("o/r", "s1").unwrap(), b"hello\n");
    let seen = h.join().unwrap();
    assert_eq!(
        seen[0].request_line,
        "GET /repos/o/r/git/trees/t?recursive=1 HTTP/1.1"
    );
}

#[test]
fn writes_send_expected_bodies() {
    let (url, h) = serve(vec![
        (201, "", r#"{"sha":"b1"}"#.into()),
        (201, "", r#"{"sha":"t1"}"#.into()),
        (201, "", r#"{"sha":"c1"}"#.into()),
        (200, "", r#"{}"#.into()),
        (
            422,
            "",
            r#"{"message":"Update is not a fast forward"}"#.into(),
        ),
        (500, "", r#"{"message":"boom"}"#.into()),
    ]);
    let api = HttpApi::with_base_url("t", url);
    assert_eq!(api.create_blob("o/r", b"hello\n").unwrap(), "b1");
    let t = api
        .create_tree(
            "o/r",
            "base",
            vec![
                NewEntry {
                    path: "a".into(),
                    sha: Some("b1".into()),
                    mode: "100644".into(),
                },
                NewEntry {
                    path: "gone".into(),
                    sha: None,
                    mode: "100644".into(),
                },
            ],
        )
        .unwrap();
    assert_eq!(t, "t1");
    assert_eq!(api.create_commit("o/r", "msg", "t1", "p").unwrap(), "c1");
    api.update_ref("o/r", "main", "c1").unwrap();
    assert_eq!(
        api.update_ref("o/r", "main", "c1"),
        Err(UpdateError::NotFastForward)
    );
    assert_eq!(
        api.update_ref("o/r", "main", "c1"),
        Err(UpdateError::Other(ApiError::Other(500, "boom".into())))
    );
    let seen = h.join().unwrap();
    let j = |i: usize| serde_json::from_str::<serde_json::Value>(&seen[i].body).unwrap();
    assert_eq!(seen[0].request_line, "POST /repos/o/r/git/blobs HTTP/1.1");
    assert_eq!(
        j(0),
        serde_json::json!({"content":"aGVsbG8K","encoding":"base64"})
    );
    assert_eq!(j(1)["base_tree"], "base");
    assert_eq!(
        j(1)["tree"][1],
        serde_json::json!({"path":"gone","mode":"100644","type":"blob","sha":null})
    );
    assert_eq!(
        j(2),
        serde_json::json!({"message":"msg","tree":"t1","parents":["p"]})
    );
    assert_eq!(
        seen[3].request_line,
        "PATCH /repos/o/r/git/refs/heads/main HTTP/1.1"
    );
    assert_eq!(j(3), serde_json::json!({"sha":"c1","force":false}));
}
