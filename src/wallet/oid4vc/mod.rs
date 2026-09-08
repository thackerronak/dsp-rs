//! OID4VC — the OpenID for Verifiable Credentials family, the wallet's second
//! credential-exchange protocol.
//!
//! Only issuance (`vci`, OID4VCI) is implemented: the connector redeems a credential
//! offer from an external issuer. Presentation over OID4VP is not implemented — the
//! wallet presents over `super::dcp` only, which is what DSP peers expect.

pub(crate) mod vci;
