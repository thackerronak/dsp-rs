#![allow(dead_code)]

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::fs;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoredCredential {
    pub(crate) id: String,
    pub(crate) credential_type: String,
    pub(crate) format: String,
    pub(crate) issuer: String,
    pub(crate) payload: String,
}

#[async_trait]
pub(crate) trait CredentialStore: Send + Sync {
    async fn store(&self, credential: &StoredCredential) -> anyhow::Result<()>;
    async fn get(&self, id: &str) -> anyhow::Result<Option<StoredCredential>>;
    async fn list(&self) -> anyhow::Result<Vec<StoredCredential>>;
    async fn delete(&self, id: &str) -> anyhow::Result<()>;
}

pub(crate) struct FileCredentialStore {
    base_path: PathBuf,
}

impl FileCredentialStore {
    pub(crate) fn new(base_path: PathBuf) -> Self {
        Self { base_path }
    }

    fn file_path(&self, id: &str) -> PathBuf {
        self.base_path.join(format!("{}.json", sanitize(id)))
    }
}

fn sanitize(id: &str) -> String {
    id.replace([':', '/', '#'], "-")
}

#[async_trait]
impl CredentialStore for FileCredentialStore {
    async fn store(&self, credential: &StoredCredential) -> anyhow::Result<()> {
        if !self.base_path.exists() {
            fs::create_dir_all(&self.base_path).await?;
        }

        let target_path = self.file_path(&credential.id);
        let temp_path = self
            .base_path
            .join(format!("{}.tmp", sanitize(&credential.id)));

        let content = serde_json::to_string_pretty(credential)?;
        fs::write(&temp_path, content).await?;
        set_permissions(&temp_path).await?;

        if let Err(err) = fs::rename(&temp_path, &target_path).await {
            let _ = fs::remove_file(&temp_path).await;
            anyhow::bail!("Atomic save failed, error: {err}");
        }

        Ok(())
    }

    async fn get(&self, id: &str) -> anyhow::Result<Option<StoredCredential>> {
        let path = self.file_path(id);
        if !path.exists() {
            return Ok(None);
        }

        let bytes = fs::read(&path).await?;
        Ok(Some(serde_json::from_slice(&bytes)?))
    }

    async fn list(&self) -> anyhow::Result<Vec<StoredCredential>> {
        let mut credentials = Vec::new();

        if !self.base_path.exists() {
            return Ok(credentials);
        }

        let mut entries = fs::read_dir(&self.base_path).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }

            let bytes = fs::read(&path).await?;
            if let Ok(credential) = serde_json::from_slice::<StoredCredential>(&bytes) {
                credentials.push(credential);
            }
        }

        Ok(credentials)
    }

    async fn delete(&self, id: &str) -> anyhow::Result<()> {
        let path = self.file_path(id);
        if path.exists() {
            fs::remove_file(path).await?;
        }
        Ok(())
    }
}

#[cfg(unix)]
async fn set_permissions(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).await?;
    Ok(())
}

#[cfg(not(unix))]
async fn set_permissions(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> StoredCredential {
        StoredCredential {
            id: "did:web:issuer-did-server#identity-1".to_string(),
            credential_type: "identity_credential".to_string(),
            format: "vc+jwt".to_string(),
            issuer: "did:web:issuer-did-server".to_string(),
            payload: "eyJhbGciOiJFUzI1NiJ9.body.sig".to_string(),
        }
    }

    #[tokio::test]
    async fn test_file_credential_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileCredentialStore::new(dir.path().to_path_buf());
        let credential = sample();

        store.store(&credential).await.unwrap();

        assert_eq!(
            store.get(&credential.id).await.unwrap(),
            Some(credential.clone())
        );

        let listed = store.list().await.unwrap();
        assert_eq!(listed, vec![credential.clone()]);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let path = dir.path().join("did-web-issuer-did-server-identity-1.json");
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        store.delete(&credential.id).await.unwrap();
        assert!(store.get(&credential.id).await.unwrap().is_none());
    }
}
