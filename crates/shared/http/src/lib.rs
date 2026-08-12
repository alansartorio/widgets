//! The one HTTP client in this workspace: nixpkgs' libcurl, through its Rust
//! bindings.
//!
//! Four integrations make HTTP requests — a Kimai instance, the Anthropic usage
//! endpoint, the feriados API and a Home Assistant — and between them they need
//! a GET with a couple of headers, a POST with a JSON body, a timeout, and the
//! status code. That is the whole surface, and it does not justify compiling an
//! HTTP stack to get it.
//!
//! Which is what the alternatives cost. `ureq` brings rustls, webpki and its own
//! protocol crate; `reqwest` brings those plus hyper, tower and an async runtime
//! — dozens of crates through the compiler on every cold build, all of it
//! reimplementing what the libcurl already in the closure does. `curl-sys` finds
//! nixpkgs' libcurl through pkg-config and links it, so the Rust side of TLS,
//! HTTP/1.1, HTTP/2, redirects and connection handling is nothing at all.
//!
//! The trade is a dynamic link against a store path instead of a static binary.
//! That is not the hazard it first looks like: the store path is in the
//! binary's closure, so it stays alive exactly as long as the binary that names
//! it, and a rebuild that moves libcurl produces a new binary alongside the old
//! rather than replacing what a running process mapped.
//!
//! A fresh handle per request, rather than a pooled agent. Nothing here polls
//! faster than once a minute except the light, which is on a LAN, so a
//! connection to reuse is worth less than a client with no shared mutable state
//! — this way a `Request` is `Send`, and no caller needs a mutex to make one.

use std::time::Duration;

use anyhow::{Context, Result, bail};

/// What a request waits before giving up, unless the caller says otherwise.
/// Short enough that a poll loop cannot stack up behind a hung server.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);

/// How much of an error body to quote back. Enough for an API's own message,
/// not enough to put a page of HTML in a log line.
const ERROR_EXCERPT: usize = 200;

/// What came back. A response is not an error here however bad the status is:
/// claude-usage tells 401 from 403 from a timeout and words the pill
/// differently for each, so the status has to survive the call.
#[derive(Clone, Debug)]
pub struct Response {
    pub status: u32,
    pub body: String,
}

impl Response {
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// The body, if the status was a 2xx, and an error naming the status and
    /// quoting the body otherwise.
    ///
    /// For the callers that have nothing useful to say about a 500 beyond
    /// passing it up — which is all of them but claude-usage.
    pub fn success(self) -> Result<String> {
        if self.is_success() {
            return Ok(self.body);
        }
        let excerpt: String = self
            .body
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(ERROR_EXCERPT)
            .collect();
        if excerpt.is_empty() {
            bail!("status {}", self.status)
        }
        bail!("status {}: {excerpt}", self.status)
    }
}

enum Body {
    None,
    /// A POST body and its content type.
    Post(Vec<u8>, &'static str),
}

pub struct Request {
    url: String,
    headers: Vec<String>,
    body: Body,
    timeout: Duration,
}

impl Request {
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            headers: Vec::new(),
            body: Body::None,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// A POST whose body is already serialized JSON.
    ///
    /// Serializing is the caller's job: this crate has no serde dependency, and
    /// the one thing it would add — a `Serialize` bound — is not worth making
    /// every user of it recompile serde's derive.
    pub fn post_json(url: impl Into<String>, body: impl Into<Vec<u8>>) -> Self {
        Self {
            body: Body::Post(body.into(), "application/json"),
            ..Self::get(url)
        }
    }

    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push(format!("{name}: {value}"));
        self
    }

    /// `Authorization: Bearer <token>`, which is what all four of these APIs
    /// want.
    pub fn bearer(self, token: &str) -> Self {
        self.header("Authorization", &format!("Bearer {token}"))
    }

    pub fn accept_json(self) -> Self {
        self.header("Accept", "application/json")
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Perform the request.
    ///
    /// An `Err` here is a transport failure — no route, no answer, a timeout, a
    /// TLS refusal. A server that answered at all comes back as [`Response`],
    /// status and body, for the caller to judge.
    pub fn send(self) -> Result<Response> {
        let mut easy = curl::easy::Easy::new();

        easy.url(&self.url)?;
        // The total budget for the request, not a per-read one: a server that
        // dribbles a byte a second must not be able to hold a poll loop open.
        easy.timeout(self.timeout)?;
        // curl does not follow redirects unless told to, and every client this
        // replaced did.
        easy.follow_location(true)?;
        easy.max_redirections(10)?;
        easy.useragent(concat!("widgets/", env!("CARGO_PKG_VERSION")))?;

        let mut headers = curl::easy::List::new();
        for header in &self.headers {
            headers.append(header)?;
        }
        if let Body::Post(body, content_type) = &self.body {
            easy.post(true)?;
            easy.post_fields_copy(body)?;
            // After the caller's headers, so an explicit Content-Type wins:
            // curl would otherwise send its form-encoded default.
            headers.append(&format!("Content-Type: {content_type}"))?;
        }
        easy.http_headers(headers)?;

        let mut body = Vec::new();
        {
            let mut transfer = easy.transfer();
            transfer.write_function(|chunk| {
                body.extend_from_slice(chunk);
                Ok(chunk.len())
            })?;
            transfer
                .perform()
                .with_context(|| format!("requesting {}", self.url))?;
        }

        Ok(Response {
            status: easy.response_code()?,
            // Lossy rather than strict: a mangled byte in an error page should
            // not turn a 500 anybody could act on into a decoding failure
            // nobody can.
            body: String::from_utf8_lossy(&body).into_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_success_hands_back_its_body() {
        let response = Response {
            status: 200,
            body: "[]".into(),
        };
        assert!(response.is_success());
        assert_eq!(response.success().unwrap(), "[]");
    }

    #[test]
    fn a_failure_names_the_status_and_quotes_the_body() {
        let error = Response {
            status: 401,
            body: "{\"message\":\"token expired\"}".into(),
        }
        .success()
        .unwrap_err()
        .to_string();
        assert!(error.contains("401"), "{error}");
        assert!(error.contains("token expired"), "{error}");
    }

    #[test]
    fn an_empty_error_body_still_says_the_status() {
        let error = Response {
            status: 502,
            body: String::new(),
        }
        .success()
        .unwrap_err()
        .to_string();
        assert_eq!(error, "status 502");
    }

    #[test]
    fn a_page_of_html_is_cut_down_to_a_line() {
        let error = Response {
            status: 500,
            body: "<html>\n  <body>\n    nope\n  </body>\n</html>".repeat(50),
        }
        .success()
        .unwrap_err()
        .to_string();
        assert!(!error.contains('\n'), "{error}");
        assert!(error.len() < ERROR_EXCERPT + 40, "{}", error.len());
    }
}
