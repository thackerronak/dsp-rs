//! Types shared between the connector and the wallet.
//!
//! The DID document is the one genuinely two-sided object: DCP advertises the credential
//! and issuer services in it, DSP advertises `CatalogService`/`DataService`.
//!
//! Nothing in here may depend on `crate::connector`, `crate::auth` or `crate::wallet`.
//! This is what keeps the dependency arrow one-way
//! (`connector -> auth -> wallet -> shared`) and makes a future crate split mechanical.

pub(crate) mod did;

pub(crate) use did::{
    CATALOG_SERVICE, CREDENTIAL_SERVICE, DATA_SERVICE, DidDocument, VERSION_ENDPOINT_PATH,
    data_service_id, derive_did_web, resolve_did_web,
};
