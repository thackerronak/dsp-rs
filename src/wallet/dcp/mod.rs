//! DCP 1.0 — the *Decentralized Claims Protocol*, one of the two credential-exchange
//! protocols this wallet speaks.
//!
//! It covers both directions: issuance (`issuer` + the holder's offer/request handling)
//! and presentation (`holder`'s Credential Service, consumed by `verifier`). The wire
//! shapes live in `model`, the authentication envelope in `si_token`, and the
//! presentation query grammar in `scope`.
//!
//! The other protocol is `super::oid4vc`, which today covers issuance only.

pub(crate) mod holder;
pub(crate) mod issuer;
pub(crate) mod model;
pub(crate) mod scope;
pub(crate) mod si_token;
pub(crate) mod sts;
pub(crate) mod verifier;

#[cfg(test)]
mod integration_issuance;
