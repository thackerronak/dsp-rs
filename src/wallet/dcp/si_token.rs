use std::{collections::HashMap, sync::Mutex};

use chrono::Utc;
use jsonwebtoken::{Algorithm, Validation, dangerous::insecure_decode, decode};
use serde::{Deserialize, Serialize};

use crate::{shared::KeyPair, wallet::did::DidResolver};

const SI_TOKEN_TTL_SECS: i64 = 300;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SiClaims {
    pub(crate) iss: String,
    pub(crate) sub: String,
    pub(crate) aud: String,
    pub(crate) iat: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) nbf: Option<i64>,
    pub(crate) exp: i64,
    pub(crate) jti: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) token: Option<String>,
}

pub(crate) fn build_si_token(
    key_pair: &KeyPair,
    iss: String,
    aud: String,
    kid: String,
    token_claim: Option<String>,
) -> anyhow::Result<String> {
    let now = Utc::now().timestamp();

    let claims = SiClaims {
        iss: iss.clone(),
        sub: iss,
        aud,
        iat: now,
        nbf: Some(now),
        exp: now + SI_TOKEN_TTL_SECS,
        jti: uuid::Uuid::new_v4().to_string(),
        token: token_claim,
    };

    key_pair.encode_with_kid(&claims, kid)
}

pub(crate) async fn validate_si_token(
    jwt: &str,
    expected_aud: &str,
    resolver: &dyn DidResolver,
    replay_cache: &ReplayCache,
) -> anyhow::Result<SiClaims> {
    let unverified = insecure_decode::<SiClaims>(jwt)?.claims;
    anyhow::ensure!(
        unverified.iss == unverified.sub,
        "SI token `iss` must equal `sub`"
    );

    let did_document = resolver.resolve(&unverified.sub).await?;
    anyhow::ensure!(
        did_document.id == unverified.sub,
        "resolved DID document does not match `sub`"
    );

    let decoding_key = did_document.capability_invocation_key()?;

    let mut validation = Validation::new(Algorithm::ES256);
    validation.set_audience(&[expected_aud]);
    validation.set_required_spec_claims(&["exp", "aud"]);

    let claims = decode::<SiClaims>(jwt, &decoding_key, &validation)?.claims;

    replay_cache.check_and_insert(&claims.jti, claims.exp)?;

    Ok(claims)
}

#[derive(Default)]
pub(crate) struct ReplayCache {
    seen: Mutex<HashMap<String, i64>>,
}

impl ReplayCache {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn check_and_insert(&self, jti: &str, exp: i64) -> anyhow::Result<()> {
        let now = Utc::now().timestamp();
        let mut guard = self.seen.lock().expect("replay cache mutex poisoned");

        guard.retain(|_, entry_exp| *entry_exp > now);

        anyhow::ensure!(
            !guard.contains_key(jti),
            "SI token replay detected for jti {jti}"
        );

        guard.insert(jti.to_string(), exp);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::DidDocument;
    use async_trait::async_trait;
    use jsonwebtoken::{EncodingKey, jwk::Jwk};
    use serde_json::{Value, json};

    const TEST_PRIVATE_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgZSL1BoLkGm8MArlQ
m60MC/g5J+38YsAycU1hukHQg3ehRANCAAQNLDb617BSV/8pn5Z/exH3sS2rMkes
5ZBcTa62LI1MRsxdz5kut+l2YWH79puf51LHNRSr35RT+smF3DcFjgg3
-----END PRIVATE KEY-----";

    const PARTY_DID: &str = "did:web:party-a";
    const VERIFIER_DID: &str = "did:web:party-b";

    struct StaticResolver {
        document: Value,
    }

    #[async_trait]
    impl DidResolver for StaticResolver {
        async fn resolve(&self, _did: &str) -> anyhow::Result<DidDocument> {
            Ok(serde_json::from_value(self.document.clone())?)
        }
    }

    fn kid() -> String {
        format!("{PARTY_DID}#keys-1")
    }

    fn resolver() -> StaticResolver {
        let encoding_key = EncodingKey::from_ec_pem(TEST_PRIVATE_KEY_PEM.as_bytes()).unwrap();
        let jwk = Jwk::from_encoding_key(&encoding_key, Algorithm::ES256).unwrap();
        let public_key_jwk = serde_json::to_value(&jwk).unwrap();

        StaticResolver {
            document: json!({
                "id": PARTY_DID,
                "verificationMethod": [{
                    "id": kid(),
                    "type": "JsonWebKey2020",
                    "controller": PARTY_DID,
                    "publicKeyJwk": public_key_jwk
                }],
                "capabilityInvocation": [kid()]
            }),
        }
    }

    #[tokio::test]
    async fn test_si_token_build_and_validate() {
        let key_pair = KeyPair::from_ec_pem(TEST_PRIVATE_KEY_PEM).unwrap();
        let token = build_si_token(
            &key_pair,
            PARTY_DID.to_string(),
            VERIFIER_DID.to_string(),
            kid(),
            Some("vp-access-token".to_string()),
        )
        .unwrap();

        let cache = ReplayCache::new();
        let claims = validate_si_token(&token, VERIFIER_DID, &resolver(), &cache)
            .await
            .expect("valid SI token");

        assert_eq!(claims.iss, PARTY_DID);
        assert_eq!(claims.sub, PARTY_DID);
        assert_eq!(claims.aud, VERIFIER_DID);
        assert_eq!(claims.token.as_deref(), Some("vp-access-token"));
    }

    #[tokio::test]
    async fn test_si_token_rejects_wrong_audience() {
        let key_pair = KeyPair::from_ec_pem(TEST_PRIVATE_KEY_PEM).unwrap();
        let token = build_si_token(
            &key_pair,
            PARTY_DID.to_string(),
            VERIFIER_DID.to_string(),
            kid(),
            None,
        )
        .unwrap();

        let cache = ReplayCache::new();
        let result = validate_si_token(&token, "did:web:someone-else", &resolver(), &cache).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_si_token_rejects_expired() {
        let key_pair = KeyPair::from_ec_pem(TEST_PRIVATE_KEY_PEM).unwrap();
        let now = Utc::now().timestamp();
        let claims = SiClaims {
            iss: PARTY_DID.to_string(),
            sub: PARTY_DID.to_string(),
            aud: VERIFIER_DID.to_string(),
            iat: now - 600,
            nbf: Some(now - 600),
            exp: now - 300,
            jti: uuid::Uuid::new_v4().to_string(),
            token: None,
        };
        let token = key_pair.encode_with_kid(&claims, kid()).unwrap();

        let cache = ReplayCache::new();
        let result = validate_si_token(&token, VERIFIER_DID, &resolver(), &cache).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_si_token_rejects_replayed_jti() {
        let key_pair = KeyPair::from_ec_pem(TEST_PRIVATE_KEY_PEM).unwrap();
        let token = build_si_token(
            &key_pair,
            PARTY_DID.to_string(),
            VERIFIER_DID.to_string(),
            kid(),
            None,
        )
        .unwrap();

        let cache = ReplayCache::new();
        validate_si_token(&token, VERIFIER_DID, &resolver(), &cache)
            .await
            .expect("first use accepted");

        let result = validate_si_token(&token, VERIFIER_DID, &resolver(), &cache).await;
        assert!(result.is_err());
    }
}
