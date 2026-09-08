//! DID resolution — protocol-agnostic.
//!
//! Both credential-exchange protocols need to turn a `did:web` into a DID document, so
//! the trait lives here rather than inside either one.

use async_trait::async_trait;
use reqwest::Client;

use crate::shared::{DidDocument, resolve_did_web};

#[async_trait]
pub(crate) trait DidResolver: Send + Sync {
    async fn resolve(&self, did: &str) -> anyhow::Result<DidDocument>;
}

pub(crate) struct HttpDidResolver {
    client: Client,
}

impl HttpDidResolver {
    pub(crate) fn new(client: Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl DidResolver for HttpDidResolver {
    async fn resolve(&self, did: &str) -> anyhow::Result<DidDocument> {
        let url = resolve_did_web(did, false)?;
        let document = self
            .client
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .json::<DidDocument>()
            .await?;

        anyhow::ensure!(document.id == did, "resolved DID document id mismatch");

        Ok(document)
    }
}
