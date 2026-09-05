//! A small HTTP server, so a command can be tested against real requests.
//!
//! The commands speak HTTP. Testing them against a mock of `reqwest` would
//! prove the mock works. This answers on a real socket, so the request is
//! built, sent, and parsed the same way it is in front of a customer.

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// One request the server saw.
#[derive(Debug, Clone)]
pub struct Seen {
    pub method: String,
    pub path: String,
    pub authorization: Option<String>,
    pub body: String,
}

impl Seen {
    /// The body as JSON, for a test that reads what was sent.
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body).unwrap_or(serde_json::Value::Null)
    }
}

/// A server that answers a scripted list, in order.
///
/// The last answer repeats once the list runs out, so a poll loop needs no
/// hundred entries.
pub struct Server {
    port: u16,
    seen: Arc<Mutex<Vec<Seen>>>,
    hits: Arc<AtomicUsize>,
}

impl Server {
    pub fn start(answers: Vec<(u16, String)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a port");
        let port = listener.local_addr().expect("an address").port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let hits = Arc::new(AtomicUsize::new(0));

        let thread_seen = Arc::clone(&seen);
        let thread_hits = Arc::clone(&hits);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let index = thread_hits.fetch_add(1, Ordering::SeqCst);
                let answer = answers
                    .get(index)
                    .or_else(|| answers.last())
                    .cloned()
                    .unwrap_or((500, "{}".to_string()));
                if let Some(request) = handle(stream, &answer) {
                    thread_seen
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push(request);
                }
            }
        });
        Self { port, seen, hits }
    }

    /// The base url a `.pekorc.json` points at.
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/v1", self.port)
    }

    pub fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }

    pub fn requests(&self) -> Vec<Seen> {
        self.seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

fn handle(mut stream: TcpStream, answer: &(u16, String)) -> Option<Seen> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut start = String::new();
    reader.read_line(&mut start).ok()?;
    let mut parts = start.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();

    let mut length = 0usize;
    let mut authorization = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 || line == "\r\n" {
            break;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(value) = lower.strip_prefix("content-length:") {
            length = value.trim().parse().unwrap_or(0);
        }
        if lower.starts_with("authorization:") {
            authorization = Some(line["authorization:".len()..].trim().to_string());
        }
    }

    let mut body = vec![0u8; length];
    if length > 0 {
        reader.read_exact(&mut body).ok()?;
    }

    let reason = if answer.0 < 400 { "OK" } else { "Error" };
    let response = format!(
        "HTTP/1.1 {} {reason}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{}",
        answer.0,
        answer.1.len(),
        answer.1
    );
    stream.write_all(response.as_bytes()).ok()?;
    stream.flush().ok()?;

    Some(Seen {
        method,
        path,
        authorization,
        body: String::from_utf8_lossy(&body).to_string(),
    })
}

/// The name of the variable that holds this project's key.
///
/// One variable per project, because an environment variable is global to
/// the process and these tests run beside each other. A shared name would let
/// one test clear the key another was using.
#[must_use]
pub fn key_var(name: &str) -> String {
    format!("PEKO_TEST_KEY_{}", name.to_uppercase().replace('-', "_"))
}

/// Write a project whose `.pekorc.json` points at the server.
/// Returns the project root and a guard.
///
/// Hold the guard for the whole test. The endpoint is a process wide variable
/// now, and without the guard a test that starts while this one is running
/// points this one at the other one's server.
pub fn project(
    name: &str,
    url: &str,
    files: &[(&str, &str)],
) -> (std::path::PathBuf, std::sync::MutexGuard<'static, ()>) {
    let root = std::env::temp_dir().join(format!(
        "peko-cli-{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let guard = ENDPOINT_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("make the project");
    for (path, body) in files {
        let target = root.join(path);
        std::fs::create_dir_all(target.parent().expect("a parent")).expect("make the directory");
        std::fs::write(&target, body).expect("write the file");
    }
    write_config_named(&root, url, &key_var(name));
    // The endpoint is process wide, because a project file that names it names
    // where the api key is sent. with_key takes the lock that keeps two tests
    // from answering each other's server.
    std::env::set_var("PEKO_API_URL", url);
    (root, guard)
}

/// Point the process at a server, for the length of one test.
///
/// The endpoint is a process wide variable now, not a field in the project
/// file, because a project file travels with somebody else's repository and
/// every request carries the api key. Tests each run their own server on a
/// random port, so they take this lock rather than overwrite each other.
pub static ENDPOINT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Write a `.pekorc.json` naming a particular key variable.
///
/// The url is not written into the file. It cannot be: a repository that
/// names the endpoint names where the api key is sent.
pub fn write_config_named(root: &std::path::Path, _url: &str, key_env: &str) {
    let config = serde_json::json!({
        "version": 1,
        "platform": "ios",
        "api_key_env": key_env,
        "facts": {},
        "overrides": [],
    });
    std::fs::write(
        root.join(".pekorc.json"),
        serde_json::to_string_pretty(&config).expect("serialize") + "\n",
    )
    .expect("write the config");
}

/// A lint answer with one error finding and one unanswered fact.
pub fn lint_answer() -> String {
    serde_json::json!({
        "tier": "lint",
        "findings": [{
            "finding_id": "00000000-0000-0000-0000-000000000001",
            "rule_id": "AAPL-API-001",
            "severity": "error",
            "title": "The app uses UIWebView",
            "message": "UIWebView matches at line 2",
            "location": {"file": "App/View.swift", "line_start": 2},
            "remediation": {"summary": "Use WKWebView"},
            "overridden": false
        }],
        "summary": {"by_severity": {"error": 1, "warning": 0, "info": 0}},
        "requests_remaining_today": 99,
        "unanswered_facts": [{"fact": "kids_category", "blocks": ["AAPL-MINOR-001"]}]
    })
    .to_string()
}
