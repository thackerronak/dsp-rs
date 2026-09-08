use anyhow::Context;
use jsonwebtoken::{DecodingKey, jwk::Jwk};
use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct VerificationMethod {
    id: String,
    controller: String,
    public_key_jwk: Jwk,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
enum VerificationRelationship {
    Reference(String),
    Embedded { id: String },
}

impl VerificationRelationship {
    fn id(&self) -> &str {
        match self {
            VerificationRelationship::Reference(id) => id,
            VerificationRelationship::Embedded { id } => id,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Service {
    #[allow(dead_code)]
    pub(crate) id: String,
    pub(crate) r#type: String,
    pub(crate) service_endpoint: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DidDocument {
    pub(crate) id: String,
    #[serde(default)]
    authentication: Vec<VerificationRelationship>,
    #[serde(default)]
    assertion_method: Vec<VerificationRelationship>,
    #[serde(default)]
    capability_invocation: Vec<VerificationRelationship>,
    #[serde(default)]
    verification_method: Vec<VerificationMethod>,
    #[serde(default)]
    pub(crate) service: Vec<Service>,
}

impl DidDocument {
    pub(crate) fn decoding_key(&self) -> anyhow::Result<DecodingKey> {
        self.decoding_key_for(&self.authentication)
    }

    #[allow(dead_code)]
    pub(crate) fn assertion_key(&self) -> anyhow::Result<DecodingKey> {
        self.decoding_key_for(&self.assertion_method)
    }

    #[allow(dead_code)]
    pub(crate) fn capability_invocation_key(&self) -> anyhow::Result<DecodingKey> {
        self.decoding_key_for(&self.capability_invocation)
    }

    #[allow(dead_code)]
    pub(crate) fn service_endpoint(&self, r#type: &str) -> Option<&str> {
        self.service
            .iter()
            .find(|service| service.r#type == r#type)
            .map(|service| service.service_endpoint.as_str())
    }

    fn decoding_key_for(
        &self,
        relationships: &[VerificationRelationship],
    ) -> anyhow::Result<DecodingKey> {
        // NOTE: This is a simplification, ideally we should use the `kid` to find the correct public key
        let Some(method) = relationships.iter().find_map(|relationship| {
            self.verification_method
                .iter()
                .find(|method| method.id == relationship.id() && method.controller == self.id)
        }) else {
            anyhow::bail!("Could not find verification method for relationship");
        };

        DecodingKey::from_jwk(&method.public_key_jwk)
            .map_err(|err| anyhow::anyhow!("Failed to decode public key, error: {err}"))
    }
}

/// DSP's version metadata endpoint, which every connector must serve.
pub(crate) const VERSION_ENDPOINT_PATH: &str = "/.well-known/dspace-version";

pub(crate) const CATALOG_SERVICE: &str = "CatalogService";
pub(crate) const DATA_SERVICE: &str = "DataService";

/// Fragment identifying this connector's `DataService` entry in its own DID document.
pub(crate) fn data_service_id(did: &str) -> String {
    format!("{did}#data-service")
}

pub(crate) fn derive_did_web(address: &str) -> anyhow::Result<String> {
    let url = Url::parse(address)
        .map_err(|err| anyhow::anyhow!("Failed to parse address, error: {err}"))?;
    let authority = url
        .host_str()
        .map(|h| match url.port() {
            Some(p) => format!("{}%3A{}", h, p),
            None => h.to_string(),
        })
        .ok_or(anyhow::anyhow!(
            "Cannot determine authority from address {}",
            address
        ))?;

    Ok(format!("did:web:{}", authority))
}

pub(crate) fn resolve_did_web(did_web: &str, enforce_https: bool) -> anyhow::Result<Url> {
    let method_specific_id = did_web
        .strip_prefix("did:web:")
        .context("Invalid did:web, missing prefix")?;

    let mut segments = method_specific_id.split(':');

    let domain = segments
        .next()
        .context("Invalid did:web, no domain specified")?
        .replace("%3A", ":")
        .replace("%3a", ":");

    let path = segments.collect::<Vec<_>>().join("/");

    let protocol = if enforce_https { "https" } else { "http" };
    let url = if path.is_empty() {
        format!("{protocol}://{domain}/.well-known/did.json")
    } else {
        format!("{protocol}://{domain}/{path}/did.json")
    };

    Ok(Url::parse(&url)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_did_document_embedded_relationship() {
        // Relationships may hold embedded `{ id, ... }` objects rather than string URIs.
        let document: DidDocument = serde_json::from_value(serde_json::json!({
            "id": "did:web:issuer-did-server",
            "verificationMethod": [{
                "id": "did:web:issuer-did-server#9vuaJyUxRx4KmHyoZ9kjJxMs_mjpnnf-mPM9nPMG51A",
                "type": "JsonWebKey2020",
                "controller": "did:web:issuer-did-server",
                "publicKeyJwk": {
                    "kty": "EC",
                    "crv": "P-256",
                    "x": "G0RINBiF-oQUD3d5DGnegQuXenI29JDaMGoMvioKRBM",
                    "y": "ed3eFGs2pEtrp7vAZ7BLcbrUtpKkYWAT2JPUQK4lN4E"
                }
            }],
            "authentication": [{ "id": "did:web:issuer-did-server#9vuaJyUxRx4KmHyoZ9kjJxMs_mjpnnf-mPM9nPMG51A" }],
            "assertionMethod": [{ "id": "did:web:issuer-did-server#9vuaJyUxRx4KmHyoZ9kjJxMs_mjpnnf-mPM9nPMG51A" }]
        }))
        .expect("did document with embedded relationships deserializes");

        assert_eq!(document.id, "did:web:issuer-did-server");
        assert!(document.decoding_key().is_ok());
        assert!(document.assertion_key().is_ok());
    }

    #[test]
    fn test_did_document_reference_relationship() {
        let document: DidDocument = serde_json::from_value(serde_json::json!({
            "id": "did:web:party",
            "verificationMethod": [{
                "id": "did:web:party#key-1",
                "type": "JsonWebKey2020",
                "controller": "did:web:party",
                "publicKeyJwk": {
                    "kty": "EC",
                    "crv": "P-256",
                    "x": "G0RINBiF-oQUD3d5DGnegQuXenI29JDaMGoMvioKRBM",
                    "y": "ed3eFGs2pEtrp7vAZ7BLcbrUtpKkYWAT2JPUQK4lN4E"
                }
            }],
            "authentication": ["did:web:party#key-1"]
        }))
        .expect("string-form did document deserializes");

        assert!(document.decoding_key().is_ok());
    }

    #[test]
    fn test_did_document_service_resolution() {
        let document: DidDocument = serde_json::from_value(serde_json::json!({
            "id": "did:web:party",
            "service": [
                {
                    "id": "did:web:party#credential-service",
                    "type": "CredentialService",
                    "serviceEndpoint": "http://party/api/credentials/v1"
                }
            ]
        }))
        .expect("did document with service deserializes");

        assert_eq!(
            document.service_endpoint("CredentialService"),
            Some("http://party/api/credentials/v1")
        );
        assert_eq!(document.service_endpoint("IssuerService"), None);
    }

    #[test]
    fn test_decoding_key_without_matching_relationship_fails() {
        let document: DidDocument = serde_json::from_value(serde_json::json!({
            "id": "did:web:party",
            "verificationMethod": [{
                "id": "did:web:party#key-1",
                "type": "JsonWebKey2020",
                "controller": "did:web:party",
                "publicKeyJwk": {
                    "kty": "EC",
                    "crv": "P-256",
                    "x": "G0RINBiF-oQUD3d5DGnegQuXenI29JDaMGoMvioKRBM",
                    "y": "ed3eFGs2pEtrp7vAZ7BLcbrUtpKkYWAT2JPUQK4lN4E"
                }
            }],
            "authentication": ["did:web:party#missing"]
        }))
        .expect("did document deserializes");

        assert!(document.decoding_key().is_err());
    }

    #[test]
    fn test_did_web_round_trip() {
        let did = derive_did_web("http://party-a-connector:3000").expect("derives");
        assert_eq!(did, "did:web:party-a-connector%3A3000");

        let url = resolve_did_web(&did, false).expect("resolves");
        assert_eq!(
            url.as_str(),
            "http://party-a-connector:3000/.well-known/did.json"
        );
    }
}
