use anyhow::Context;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode, jwk::Jwk};
use serde::Serialize;

/// The wallet's signing key. Signs anything a peer verifies, so its public half is
/// published in the DID document.
#[derive(Clone)]
pub(crate) struct KeyPair {
    encoding_key: EncodingKey,
    public_jwk: Jwk,
}

impl KeyPair {
    pub(crate) fn from_ec_pem(private_key_pem: &str) -> anyhow::Result<Self> {
        let encoding_key = EncodingKey::from_ec_pem(private_key_pem.as_bytes())
            .context("Failed to decode private key format")?;
        let jwk = Jwk::from_encoding_key(&encoding_key, Algorithm::ES256)
            .context("Failed to convert encoding key to jwk")?;

        Ok(Self {
            encoding_key,
            public_jwk: jwk,
        })
    }

    pub(crate) fn public_jwk(&self) -> &Jwk {
        &self.public_jwk
    }

    pub(crate) fn encode_with_kid<T: Serialize>(
        &self,
        claims: T,
        kid: String,
    ) -> anyhow::Result<String> {
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some(kid);
        encode(&header, &claims, &self.encoding_key).map_err(anyhow::Error::msg)
    }

    /// Sign with a caller-supplied header.
    ///
    /// Needed where the `typ` matters — an OID4VCI proof JWT must carry
    /// `typ: "openid4vci-proof+jwt"`, not the default `JWT`.
    pub(crate) fn encode_with_header<T: Serialize>(
        &self,
        claims: T,
        kid: String,
        typ: &str,
    ) -> anyhow::Result<String> {
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some(kid);
        header.typ = Some(typ.to_string());
        encode(&header, &claims, &self.encoding_key).map_err(anyhow::Error::msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const TEST_PRIVATE_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgZSL1BoLkGm8MArlQ
m60MC/g5J+38YsAycU1hukHQg3ehRANCAAQNLDb617BSV/8pn5Z/exH3sS2rMkes
5ZBcTa62LI1MRsxdz5kut+l2YWH79puf51LHNRSr35RT+smF3DcFjgg3
-----END PRIVATE KEY-----";

    #[test]
    fn test_encode_with_kid() {
        let key_pair = KeyPair::from_ec_pem(TEST_PRIVATE_KEY_PEM).expect("valid test key");
        let did = "did:web:party-a";
        let kid = format!("{did}#keys-1");

        let token = key_pair
            .encode_with_kid(json!({ "sub": did }), kid.clone())
            .expect("token encodes");

        let header = jsonwebtoken::decode_header(&token).expect("header decodes");
        assert_eq!(header.kid, Some(kid));
    }
}
