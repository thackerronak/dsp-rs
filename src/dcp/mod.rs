#![allow(dead_code)]

use axum::http::{HeaderMap, header::AUTHORIZATION};

pub(crate) mod holder;
pub(crate) mod issuer;
pub(crate) mod model;
pub(crate) mod resolver;
pub(crate) mod scope;
pub(crate) mod si_token;
pub(crate) mod sts;
pub(crate) mod verifier;

#[cfg(test)]
mod integration_issuance;
#[cfg(test)]
pub(crate) mod test_support;

pub(crate) fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            value
                .strip_prefix("Bearer ")
                .or_else(|| value.strip_prefix("bearer "))
        })
}
