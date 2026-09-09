use jsonwebtoken::{Algorithm, Validation, dangerous::insecure_decode, decode};
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;

use crate::{
    shared::DidDocument,
    wallet::{
        KeyPair,
        dcp::{
            model::{DCP_CONTEXT, PresentationQueryMessage, PresentationResponseMessage},
            scope::IDENTITY_CREDENTIAL_SCOPE,
            si_token::build_si_token,
        },
        did::DidResolver,
    },
};

#[derive(Deserialize)]
struct UnverifiedClaims {
    iss: String,
}

pub(crate) async fn query_peer_presentation(
    client: &Client,
    key_pair: &KeyPair,
    local_did: &str,
    kid: &str,
    peer_did: &str,
    credential_service_endpoint: &str,
    forward_token: Option<String>,
) -> anyhow::Result<String> {
    let si_token = build_si_token(
        key_pair,
        local_did.to_string(),
        peer_did.to_string(),
        kid.to_string(),
        forward_token,
    )?;

    let query = PresentationQueryMessage {
        context: vec![DCP_CONTEXT.to_string()],
        message_type: "PresentationQueryMessage".to_string(),
        scope: Some(vec![IDENTITY_CREDENTIAL_SCOPE.to_string()]),
        presentation_definition: None,
    };

    let url = format!("{credential_service_endpoint}/presentations/query");
    let response = client
        .post(&url)
        .bearer_auth(si_token)
        .json(&query)
        .send()
        .await?
        .error_for_status()?;

    let resp_msg: PresentationResponseMessage = response.json().await?;
    let vp_value = resp_msg
        .presentation
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("peer returned empty presentation list"))?;

    match vp_value {
        Value::String(s) => Ok(s),
        other => anyhow::bail!("expected VP as string, got {other}"),
    }
}

pub(crate) fn validate_vp(
    vp_jwt: &str,
    expected_holder_did: &str,
    expected_verifier_did: &str,
    holder_doc: &DidDocument,
) -> anyhow::Result<String> {
    let decoding_key = holder_doc
        .decoding_key()
        .or_else(|_| holder_doc.assertion_key())?;

    let mut validation = Validation::new(Algorithm::ES256);
    validation.set_audience(&[expected_verifier_did]);
    validation.set_issuer(&[expected_holder_did]);
    // `sub` is not required on a presentation: the holder presents it, so the binding is
    // `iss`, which is checked above. Implementations differ here — the Java EDC omits
    // `sub` entirely — so only check it when the holder chose to send one.
    validation.set_required_spec_claims(&["exp", "aud", "iss"]);

    let token_data = decode::<Value>(vp_jwt, &decoding_key, &validation)?;
    let claims = token_data.claims;

    if let Some(sub) = claims.get("sub").and_then(Value::as_str) {
        anyhow::ensure!(
            sub == expected_holder_did,
            "VP sub `{sub}` does not match expected holder `{expected_holder_did}`"
        );
    }

    let vc_array = claims
        .get("vp")
        .and_then(|vp| vp.get("verifiableCredential"))
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("missing vp.verifiableCredential in VP"))?;

    anyhow::ensure!(
        !vc_array.is_empty(),
        "vp.verifiableCredential array is empty"
    );

    let vc_jwt = vc_array[0]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("verifiableCredential[0] is not a string"))?;

    Ok(vc_jwt.to_string())
}

pub(crate) async fn validate_vc(
    vc_jwt: &str,
    expected_holder_did: &str,
    allowed_issuers: &[String],
    resolver: &dyn DidResolver,
) -> anyhow::Result<(String, Value)> {
    let unverified = insecure_decode::<UnverifiedClaims>(vc_jwt)?;
    let issuer = unverified.claims.iss;

    if !allowed_issuers.is_empty() && !allowed_issuers.contains(&issuer) {
        anyhow::bail!("VC issuer `{issuer}` is not in allowed_issuers");
    }

    let issuer_doc = resolver.resolve(&issuer).await?;
    let decoding_key = issuer_doc
        .assertion_key()
        .or_else(|_| issuer_doc.decoding_key())?;

    let mut validation = Validation::new(Algorithm::ES256);
    validation.set_issuer(&[&issuer]);
    validation.set_required_spec_claims(&["exp", "iss", "sub"]);

    let token_data = decode::<Value>(vc_jwt, &decoding_key, &validation)?;
    let claims = token_data.claims;

    let sub = claims
        .get("sub")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("missing sub in VC claims"))?;
    anyhow::ensure!(
        sub == expected_holder_did,
        "VC sub `{sub}` does not match expected holder `{expected_holder_did}`"
    );

    let subject = claims
        .get("vc")
        .and_then(|vc| vc.get("credentialSubject"))
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("missing vc.credentialSubject in VC"))?;

    Ok((issuer, subject))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wallet::{
        dcp::{issuer::mint_identity_credential, scope::mint_jwt_vp},
        test_support::{HOLDER_DID, ISSUER_DID, kid, static_resolver, test_key_pair},
    };
    use serde_json::json;

    #[tokio::test]
    async fn test_validate_vp_and_vc_happy() {
        let key_pair = test_key_pair();
        let resolver = static_resolver();

        let subject = json!({
            "email": "ops@example.com",
            "address": { "country": "DE" }
        });

        let vc = mint_identity_credential(
            &key_pair,
            ISSUER_DID,
            &kid(ISSUER_DID),
            HOLDER_DID,
            subject.clone(),
        )
        .unwrap();

        let vp = mint_jwt_vp(&key_pair, HOLDER_DID, &kid(HOLDER_DID), ISSUER_DID, &vc).unwrap();

        let holder_doc = resolver.resolve(HOLDER_DID).await.unwrap();
        let extracted_vc = validate_vp(&vp, HOLDER_DID, ISSUER_DID, &holder_doc).unwrap();
        assert_eq!(extracted_vc, vc);

        let allowed_issuers = vec![ISSUER_DID.to_string()];
        let (issuer, subj) = validate_vc(&extracted_vc, HOLDER_DID, &allowed_issuers, &resolver)
            .await
            .unwrap();
        assert_eq!(issuer, ISSUER_DID);
        // The issuer stamps the holder into the subject on the way out.
        assert_eq!(subj["email"], subject["email"]);
        assert_eq!(subj["address"], subject["address"]);
        assert_eq!(subj["id"], HOLDER_DID);
    }

    /// The Java EDC's presentations carry no `sub`; the holder is `iss`.
    #[tokio::test]
    async fn test_validate_vp_accepts_a_presentation_without_sub() {
        let key_pair = test_key_pair();
        let resolver = static_resolver();

        let vc = mint_identity_credential(
            &key_pair,
            ISSUER_DID,
            &kid(ISSUER_DID),
            HOLDER_DID,
            json!({ "email": "ops@example.com", "address": { "country": "DE" } }),
        )
        .unwrap();

        let now = chrono::Utc::now().timestamp();
        let vp = key_pair
            .encode_with_kid(
                json!({
                    "iss": HOLDER_DID,
                    "aud": ISSUER_DID,
                    "iat": now,
                    "nbf": now,
                    "exp": now + 300,
                    "jti": uuid::Uuid::new_v4().to_string(),
                    "vp": {
                        "@context": ["https://www.w3.org/2018/credentials/v1"],
                        "type": ["VerifiablePresentation"],
                        "verifiableCredential": [vc.clone()]
                    }
                }),
                kid(HOLDER_DID),
            )
            .unwrap();

        let holder_doc = resolver.resolve(HOLDER_DID).await.unwrap();
        let extracted = validate_vp(&vp, HOLDER_DID, ISSUER_DID, &holder_doc).unwrap();
        assert_eq!(extracted, vc);
    }

    /// A `sub` that disagrees with the holder is still a forgery.
    #[tokio::test]
    async fn test_validate_vp_rejects_a_mismatched_sub() {
        let key_pair = test_key_pair();
        let resolver = static_resolver();

        let vc = mint_identity_credential(
            &key_pair,
            ISSUER_DID,
            &kid(ISSUER_DID),
            HOLDER_DID,
            json!({ "email": "ops@example.com" }),
        )
        .unwrap();

        let vp = mint_jwt_vp(&key_pair, HOLDER_DID, &kid(HOLDER_DID), ISSUER_DID, &vc).unwrap();
        let holder_doc = resolver.resolve(HOLDER_DID).await.unwrap();

        // Presented as if it belonged to a third party.
        assert!(validate_vp(&vp, "did:web:someone-else", ISSUER_DID, &holder_doc).is_err());
    }

    #[tokio::test]
    async fn test_validate_vc_rejects_untrusted_issuer() {
        let key_pair = test_key_pair();
        let resolver = static_resolver();

        let subject = json!({
            "email": "ops@example.com",
            "address": { "country": "DE" }
        });

        let vc =
            mint_identity_credential(&key_pair, ISSUER_DID, &kid(ISSUER_DID), HOLDER_DID, subject)
                .unwrap();

        let allowed_issuers = vec!["did:web:untrusted-issuer".to_string()];
        let result = validate_vc(&vc, HOLDER_DID, &allowed_issuers, &resolver).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("not in allowed_issuers")
        );
    }
}
