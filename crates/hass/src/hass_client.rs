use reqwest::{Client, Result};
use serde_json::Value;
use url::Url;

#[derive(Debug)]
pub struct HassClient {
    uri: Url,
    token: String,
    http_client: Client,
}

impl HassClient {
    pub fn new(uri: Url, token: String) -> Self {
        Self {
            uri,
            token,
            http_client: Client::new(),
        }
    }

    pub async fn get_state(&self, entity_id: &str) -> Result<Value> {
        let HassClient { uri, token, .. } = self;

        self.http_client
            .get(uri.join(&format!("/api/states/{entity_id}")).unwrap())
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await?
            .text()
            .await
            .map(|t| serde_json::from_str(&t).unwrap())
    }

    pub async fn set_state(&self, domain: &str, service: &str, value: Value) -> Result<Value> {
        let HassClient { uri, token, .. } = self;

        self.http_client
            .post(
                uri.join(&format!("/api/services/{domain}/{service}"))
                    .unwrap(),
            )
            .json(&value)
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await?
            .json()
            .await
    }
}
