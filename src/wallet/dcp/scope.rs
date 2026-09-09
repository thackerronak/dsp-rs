use serde_json::json;

use crate::wallet::KeyPair;

const SCOPE_TYPE_PREFIX: &str = "org.eclipse.dspace.dcp.vc.type:";
const SCOPE_ID_PREFIX: &str = "org.eclipse.dspace.dcp.vc.id:";
const PRESENTATION_VALIDITY_SECS: i64 = 300;

/// Operations a type scope may end in. An id scope carries none, because credential ids
/// are URNs and contain colons of their own.
const SCOPE_OPERATIONS: [&str; 3] = ["read", "*", "all"];

/// The scope this connector asks a peer's Credential Service for. The trailing operation
/// is required by the scope grammar EDC implements, and is ignored by peers that only
/// match on the credential type.
pub(crate) const IDENTITY_CREDENTIAL_SCOPE: &str =
    "org.eclipse.dspace.dcp.vc.type:identity_credential:read";

#[derive(Debug, PartialEq, Eq, Clone)]
pub(crate) enum ScopeQuery {
    Type(String),
    Id(String),
}

impl ScopeQuery {
    pub(crate) fn parse(scope: &str) -> Option<Self> {
        if let Some(rest) = scope.strip_prefix(SCOPE_TYPE_PREFIX) {
            let credential_type = strip_operation(rest);
            if !credential_type.is_empty() {
                return Some(Self::Type(credential_type.to_string()));
            }
        } else if let Some(rest) = scope.strip_prefix(SCOPE_ID_PREFIX)
            && !rest.is_empty()
        {
            return Some(Self::Id(rest.to_string()));
        }
        None
    }
}

fn strip_operation(rest: &str) -> &str {
    match rest.rsplit_once(':') {
        Some((credential_type, operation)) if SCOPE_OPERATIONS.contains(&operation) => {
            credential_type
        }
        _ => rest,
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
    use crate::wallet::test_support::{HOLDER_DID, ISSUER_DID, kid, test_key_pair};
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
    fn test_scope_query_parsing_strips_the_operation() {
        for scope in [
            "org.eclipse.dspace.dcp.vc.type:identity_credential:read",
            "org.eclipse.dspace.dcp.vc.type:identity_credential:*",
            "org.eclipse.dspace.dcp.vc.type:identity_credential:all",
        ] {
            assert_eq!(
                ScopeQuery::parse(scope),
                Some(ScopeQuery::Type("identity_credential".to_string())),
                "{scope}"
            );
        }

        // Only a known operation is stripped, so a type containing a colon survives.
        assert_eq!(
            ScopeQuery::parse("org.eclipse.dspace.dcp.vc.type:example.org:MembershipCredential"),
            Some(ScopeQuery::Type(
                "example.org:MembershipCredential".to_string()
            ))
        );

        // Id scopes take no operation, so a URN keeps every colon it has.
        assert_eq!(
            ScopeQuery::parse("org.eclipse.dspace.dcp.vc.id:urn:uuid:1234:read"),
            Some(ScopeQuery::Id("urn:uuid:1234:read".to_string()))
        );
    }

    #[test]
    fn test_requested_scope_carries_an_operation() {
        // EDC's Credential Service rejects a type scope with no operation.
        assert_eq!(
            ScopeQuery::parse(IDENTITY_CREDENTIAL_SCOPE),
            Some(ScopeQuery::Type("identity_credential".to_string()))
        );
        assert!(IDENTITY_CREDENTIAL_SCOPE.ends_with(":read"));
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
