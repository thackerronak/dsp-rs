
use axum::http::{HeaderMap, header::AUTHORIZATION};

pub(crate) mod holder;
pub(crate) mod issuer;
pub(crate) mod model;
pub(crate) mod resolver;
pub(crate) mod scope;
pub(crate) mod si_token;
pub(crate) mod store;
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

#[cfg(test)]
mod boundary {
    /// `crate::dcp` must depend only on `crate::shared` and external crates.
    ///
    /// This is what keeps the arrow one-way (`connector -> auth -> dcp -> shared`) and makes
    /// lifting the wallet into its own crate — or its own process — a move rather than a
    /// redesign. If this fails, something reached back into the connector or the auth layer.
    #[test]
    fn dcp_does_not_depend_on_connector_or_auth() {
        // Built at runtime so this test does not match its own source.
        let forbidden: Vec<String> = ["connector", "auth"]
            .iter()
            .flat_map(|layer| [format!("crate::{layer}::"), format!("{layer}::")])
            .collect();

        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/dcp");
        let mut violations = Vec::new();

        for entry in std::fs::read_dir(&dir).expect("src/dcp is readable") {
            let path = entry.expect("readable entry").path();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }

            let source = std::fs::read_to_string(&path).expect("source is readable");
            let name = path.file_name().unwrap().to_string_lossy().to_string();

            for (number, line) in source.lines().enumerate() {
                let code = line.split("//").next().unwrap_or(line);
                if forbidden.iter().any(|pattern| code.contains(pattern.as_str())) {
                    violations.push(format!("{name}:{}: {}", number + 1, code.trim()));
                }
            }
        }

        assert!(
            violations.is_empty(),
            "src/dcp must not depend on the connector or auth layers:\n{}",
            violations.join("\n")
        );
    }
}
