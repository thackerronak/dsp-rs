use std::any::Any;

use async_trait::async_trait;
use axum::Router;
use reqwest::Client;
use serde_json::Value;

use crate::connector::app_state::AppStateAuthentication;

#[async_trait]
pub(crate) trait AuthBackend: Send + Sync {
    async fn get_token(
        &self,
        client: &Client,
        remote_address: &str,
        did_web: String,
    ) -> anyhow::Result<String>;

    async fn did_document(&self, client: &Client, did: &str) -> anyhow::Result<Value>;

    fn router(&self) -> Router<AppStateAuthentication>;

    fn extra_routes(&self) -> Option<Router> {
        None
    }

    fn as_any(&self) -> &dyn Any;
}
