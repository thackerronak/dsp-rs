use serde_json::json;

use crate::connector::app_state::KeyPair;

const SCOPE_TYPE_PREFIX: &str = "org.eclipse.dspace.dcp.vc.type:";
const SCOPE_ID_PREFIX: &str = "org.eclipse.dspace.dcp.vc.id:";
const PRESENTATION_VALIDITY_SECS: i64 = 300;

#[derive(Debug, PartialEq, Eq, Clone)]
pub(crate) enum ScopeQuery {
    Type(String),
    Id(String),
}

impl ScopeQuery {
    pub(crate) fn parse(scope: &str) -> Option<Self> {
        if let Some(rest) = scope.strip_prefix(SCOPE_TYPE_PREFIX) {
            if !rest.is_empty() {
                return Some(Self::Type(rest.to_string()));
            }
        } else if let Some(rest) = scope.strip_prefix(SCOPE_ID_PREFIX) {
            if !rest.is_empty() {
                return Some(Self::Id(rest.to_string()));
            }
        }
        None
    }
}

pub(crate) fn mint_jwt_vp(
    key_pair: &KeyPair,
    holder_did: &str,
    kid: &str,
    verifier_did: &str,
    vc_jwt: &str,
) -> anyhow::Result<String> {
    let now = chrono::Utc::now().timestamp();

    let claims = json!({
        "iss": holder_did,
        "sub": holder_did,
        "aud": verifier_did,
        "iat": now,
        "nbf": now,
        "exp": now + PRESENTATION_VALIDITY_SECS,
        "jti": uuid::Uuid::new_v4().to_string(),
        "vp": {
            "@context": ["https://www.w3.org/2018/credentials/v1"],
            "type": ["VerifiablePresentation"],
            "verifiableCredential": [vc_jwt]
        }
    });

    key_pair.encode_with_kid(claims, kid.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dcp::test_support::{HOLDER_DID, ISSUER_DID, kid, test_key_pair};
    use jsonwebtoken::dangerous::insecure_decode;
    use serde_json::Value;

    #[test]
    fn test_scope_query_parsing() {
        assert_eq!(
            ScopeQuery::parse("org.eclipse.dspace.dcp.vc.type:identity_credential"),
            Some(ScopeQuery::Type("identity_credential".to_string()))
        );
        assert_eq!(
            ScopeQuery::parse("org.eclipse.dspace.dcp.vc.id:cred-uuid-123"),
            Some(ScopeQuery::Id("cred-uuid-123".to_string()))
        );
        assert_eq!(ScopeQuery::parse("org.eclipse.dspace.dcp.vc.type:"), None);
        assert_eq!(ScopeQuery::parse("unknown:scope"), None);
    }

    #[test]
    fn test_mint_jwt_vp() {
        let key_pair = test_key_pair();
        let holder_did = HOLDER_DID;
        let verifier_did = ISSUER_DID;
        let holder_kid = kid(holder_did);
        let vc_payload = "dummy.vc.jwt";

        let vp = mint_jwt_vp(&key_pair, holder_did, &holder_kid, verifier_did, vc_payload)
            .expect("mints jwt-vp");

        let decoded = insecure_decode::<Value>(&vp).expect("decodes vp");
        let claims = decoded.claims;
        assert_eq!(claims["iss"], holder_did);
        assert_eq!(claims["sub"], holder_did);
        assert_eq!(claims["aud"], verifier_did);

        let vp_obj = &claims["vp"];
        assert_eq!(
            vp_obj["type"].as_array().unwrap(),
            &vec![json!("VerifiablePresentation")]
        );
        assert_eq!(
            vp_obj["verifiableCredential"].as_array().unwrap(),
            &vec![json!(vc_payload)]
        );
    }
}
