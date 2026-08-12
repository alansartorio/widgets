use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::Value;

/// Well under anything that waits on it, so a hung request cannot pin the
/// thread that made it.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug)]
pub struct HassClient {
    base: String,
    token: String,
}

impl HassClient {
    pub fn new(base: &str, token: String) -> Self {
        Self {
            // Trailing slashes would double up in every request path. The
            // version this replaces used `Url::join`, which does more than that
            // and less usefully: joining "/api/…" onto a base *discards* any
            // path the base had, so a Home Assistant behind a reverse proxy at
            // /hass would have been asked the wrong URL.
            base: base.trim_end_matches('/').to_string(),
            token,
        }
    }

    pub fn get_state(&self, entity_id: &str) -> Result<Value> {
        let body = widgets_http::Request::get(format!("{}/api/states/{entity_id}", self.base))
            .bearer(&self.token)
            .accept_json()
            .timeout(REQUEST_TIMEOUT)
            .send()
            .with_context(|| format!("asking Home Assistant about {entity_id}"))?
            .success()?;
        serde_json::from_str(&body).with_context(|| format!("unreadable state for {entity_id}"))
    }

    /// Call a service. The answer is the list of states Home Assistant changed,
    /// which the caller already knows, so only the status is read.
    pub fn set_state(&self, domain: &str, service: &str, value: &Value) -> Result<()> {
        let payload = serde_json::to_string(value).expect("a service call is always serialisable");
        widgets_http::Request::post_json(
            format!("{}/api/services/{domain}/{service}", self.base),
            payload,
        )
        .bearer(&self.token)
        .timeout(REQUEST_TIMEOUT)
        .send()
        .with_context(|| format!("calling {domain}.{service}"))?
        .success()?;
        Ok(())
    }
}
