use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A credential as handed from the verifier to `Authenticator::derive_access_token`.
///
/// The DID-document types this module used to carry now live in `crate::shared::did`,
/// so that `crate::dcp` can use them without depending on the auth layer.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct CredentialData {
    #[serde(rename = "type")]
    pub(crate) r#type: String,

    pub(crate) format: String,

    #[serde(rename = "credentialData")]
    pub(crate) credential_data: Value,

    pub(crate) issuer: String,
}
