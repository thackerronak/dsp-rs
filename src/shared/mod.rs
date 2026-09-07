//! Types shared between the connector and the DCP wallet.
//!
//! Nothing in here may depend on `crate::connector`, `crate::auth` or `crate::dcp`.
//! This is what keeps the dependency arrow one-way
//! (`connector -> auth -> dcp -> shared`) and makes a future crate split mechanical.

pub(crate) mod did;
pub(crate) mod key_pair;

pub(crate) use did::{DidDocument, derive_did_web, resolve_did_web};
pub(crate) use key_pair::KeyPair;
