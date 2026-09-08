use serde::{Deserialize, Serialize};
use serde_json::Value;

pub(crate) const DCP_CONTEXT: &str = "https://w3id.org/dspace-dcp/v1.0/dcp.jsonld";

fn dcp_context() -> Vec<String> {
    vec![DCP_CONTEXT.to_string()]
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CredentialObject {
    pub(crate) id: String,
    pub(crate) credential_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) profile: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) binding_methods: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) schema: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CredentialContainer {
    pub(crate) credential_type: String,
    pub(crate) payload: String,
    pub(crate) format: String,
    pub(crate) r#type: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CredentialReference {
    pub(crate) id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CredentialOfferMessage {
    #[serde(rename = "@context", default = "dcp_context")]
    pub(crate) context: Vec<String>,
    #[serde(rename = "type")]
    pub(crate) message_type: String,
    pub(crate) issuer: String,
    pub(crate) credentials: Vec<CredentialObject>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CredentialRequestMessage {
    #[serde(rename = "@context", default = "dcp_context")]
    pub(crate) context: Vec<String>,
    #[serde(rename = "type")]
    pub(crate) message_type: String,
    pub(crate) holder_pid: String,
    pub(crate) credentials: Vec<CredentialReference>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CredentialMessage {
    #[serde(rename = "@context", default = "dcp_context")]
    pub(crate) context: Vec<String>,
    #[serde(rename = "type")]
    pub(crate) message_type: String,
    pub(crate) issuer_pid: String,
    pub(crate) holder_pid: String,
    pub(crate) status: String,
    pub(crate) credentials: Vec<CredentialContainer>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CredentialRequestStatusMessage {
    #[serde(rename = "@context", default = "dcp_context")]
    pub(crate) context: Vec<String>,
    #[serde(rename = "type")]
    pub(crate) message_type: String,
    pub(crate) issuer_pid: String,
    pub(crate) holder_pid: String,
    pub(crate) status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IssuerMetadata {
    pub(crate) issuer: String,
    pub(crate) credentials_supported: Vec<CredentialObject>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PresentationQueryMessage {
    #[serde(rename = "@context", default = "dcp_context")]
    pub(crate) context: Vec<String>,
    #[serde(rename = "type")]
    pub(crate) message_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) scope: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) presentation_definition: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PresentationResponseMessage {
    #[serde(rename = "@context", default = "dcp_context")]
    pub(crate) context: Vec<String>,
    #[serde(rename = "type")]
    pub(crate) message_type: String,
    pub(crate) presentation: Vec<Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn round_trip<T>(value: &Value)
    where
        T: Serialize + for<'de> Deserialize<'de>,
    {
        let parsed: T = serde_json::from_value(value.clone()).expect("deserializes");
        let serialized = serde_json::to_value(&parsed).expect("serializes");
        assert_eq!(&serialized, value);
    }

    #[test]
    fn test_dcp_models_credential_offer() {
        round_trip::<CredentialOfferMessage>(&json!({
            "@context": [DCP_CONTEXT],
            "type": "CredentialOfferMessage",
            "issuer": "did:web:issuer",
            "credentials": [{
                "id": "credential-1",
                "credentialType": "identity_credential"
            }]
        }));
    }

    #[test]
    fn test_dcp_models_credential_request() {
        round_trip::<CredentialRequestMessage>(&json!({
            "@context": [DCP_CONTEXT],
            "type": "CredentialRequestMessage",
            "holderPid": "holder-pid-1",
            "credentials": [{ "id": "credential-1" }]
        }));
    }

    #[test]
    fn test_dcp_models_credential_message() {
        round_trip::<CredentialMessage>(&json!({
            "@context": [DCP_CONTEXT],
            "type": "CredentialMessage",
            "issuerPid": "issuer-pid-1",
            "holderPid": "holder-pid-1",
            "status": "ISSUED",
            "credentials": [{
                "credentialType": "identity_credential",
                "payload": "eyJ...",
                "format": "vc+jwt",
                "type": "CredentialContainer"
            }]
        }));
    }

    #[test]
    fn test_dcp_models_presentation_query_scope() {
        round_trip::<PresentationQueryMessage>(&json!({
            "@context": [DCP_CONTEXT],
            "type": "PresentationQueryMessage",
            "scope": ["org.eclipse.dspace.dcp.vc.type:identity_credential"]
        }));
    }

    #[test]
    fn test_dcp_models_presentation_response() {
        round_trip::<PresentationResponseMessage>(&json!({
            "@context": [DCP_CONTEXT],
            "type": "PresentationResponseMessage",
            "presentation": ["eyJ..."]
        }));
    }

    #[test]
    fn test_dcp_models_context_defaults_when_absent() {
        let message: PresentationQueryMessage = serde_json::from_value(json!({
            "type": "PresentationQueryMessage",
            "scope": ["org.eclipse.dspace.dcp.vc.type:identity_credential"]
        }))
        .expect("deserializes without @context");

        assert_eq!(message.context, vec![DCP_CONTEXT.to_string()]);
    }
}
