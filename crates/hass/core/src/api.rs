//! A client for the two Home Assistant endpoints this needs.
//!
//! `GET /api/states/<entity_id>` for what the lamp is doing, and
//! `POST /api/services/<domain>/<service>` to change it. Both want a long-lived
//! access token as `Authorization: Bearer <token>` (profile → security →
//! long-lived access tokens).

use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::Value;

/// What the config defaults to: well under anything that waits on a request, so
/// a hung one cannot pin the thread that made it.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug)]
pub struct Client {
    base: String,
    token: String,
    timeout: Duration,
}

impl Client {
    pub fn new(base: &str, token: String, timeout: Duration) -> Self {
        Self {
            // Trailing slashes would double up in every request path.
            base: base.trim_end_matches('/').to_string(),
            token,
            timeout,
        }
    }

    /// One entity's whole state object: `state`, `attributes` and all.
    pub fn state(&self, entity_id: &str) -> Result<Value> {
        let body = widgets_http::Request::get(format!("{}/api/states/{entity_id}", self.base))
            .bearer(&self.token)
            .accept_json()
            .timeout(self.timeout)
            .send()
            .with_context(|| format!("asking Home Assistant about {entity_id}"))?
            .success()?;
        serde_json::from_str(&body).with_context(|| format!("unreadable state for {entity_id}"))
    }

    /// Call a service. The answer is the list of states Home Assistant changed,
    /// which the caller already knows, so only the status is read.
    pub fn call_service(&self, domain: &str, service: &str, body: &Value) -> Result<()> {
        let payload = serde_json::to_string(body).expect("a service call is always serialisable");
        widgets_http::Request::post_json(
            format!("{}/api/services/{domain}/{service}", self.base),
            payload,
        )
        .bearer(&self.token)
        .timeout(self.timeout)
        .send()
        .with_context(|| format!("calling {domain}.{service}"))?
        .success()?;
        Ok(())
    }
}
