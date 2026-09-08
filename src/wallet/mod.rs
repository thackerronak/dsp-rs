//! The connector's wallet: it holds credentials, presents them, and issues them.
//!
//! The wallet is protocol-agnostic at its core — `store` keeps the credentials and `did`
//! resolves peers. Credential *exchange* is delegated to a protocol submodule:
//!
//! - `dcp` — Decentralized Claims Protocol 1.0: issuance and presentation
//! - `oid4vc` — OpenID for Verifiable Credentials: issuance only (OID4VCI)
//!
//! Nothing here may depend on `crate::connector` or `crate::auth`; see the boundary test
//! at the bottom of this file.

use axum::http::{HeaderMap, header::AUTHORIZATION};

pub(crate) mod dcp;
pub(crate) mod did;
pub(crate) mod key_pair;
pub(crate) mod oid4vc;
pub(crate) mod store;

pub(crate) use key_pair::KeyPair;

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

#[cfg(test)]
mod boundary {
    /// `crate::wallet` must depend only on `crate::shared` and external crates.
    ///
    /// This is what keeps the arrow one-way (`connector -> auth -> wallet -> shared`) and
    /// makes lifting the wallet into its own crate — or its own process — a move rather
    /// than a redesign. If this fails, something reached back into the connector or the
    /// auth layer.
    #[test]
    fn wallet_does_not_depend_on_connector_or_auth() {
        // Built at runtime so this test does not match its own source.
        let forbidden: Vec<String> = ["connector", "auth"]
            .iter()
            .flat_map(|layer| [format!("crate::{layer}::"), format!("{layer}::")])
            .collect();

        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/wallet");
        let mut sources = Vec::new();
        collect_sources(&root, &root, &mut sources);

        assert!(
            !sources.is_empty(),
            "found no sources under {}",
            root.display()
        );

        let mut violations = Vec::new();

        for (name, source) in sources {
            for (number, line) in source.lines().enumerate() {
                let code = line.split("//").next().unwrap_or(line);
                if forbidden
                    .iter()
                    .any(|pattern| code.contains(pattern.as_str()))
                {
                    violations.push(format!("{name}:{}: {}", number + 1, code.trim()));
                }
            }
        }

        assert!(
            violations.is_empty(),
            "src/wallet must not depend on the connector or auth layers:\n{}",
            violations.join("\n")
        );
    }

    /// Every `.rs` file under `dir`, recursively, as `(path relative to root, contents)`.
    fn collect_sources(
        root: &std::path::Path,
        dir: &std::path::Path,
        out: &mut Vec<(String, String)>,
    ) {
        for entry in std::fs::read_dir(dir).expect("wallet directory is readable") {
            let path = entry.expect("readable entry").path();

            if path.is_dir() {
                collect_sources(root, &path, out);
                continue;
            }

            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }

            let source = std::fs::read_to_string(&path).expect("source is readable");
            let name = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();

            out.push((name, source));
        }
    }
}
