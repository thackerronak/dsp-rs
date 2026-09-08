use std::time::Duration;

use anyhow::Context;
use tokio::{
    select,
    time::{MissedTickBehavior, interval},
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::{
    connector::{
        app_state::{AppStateCatalog, RemoteConnector},
        get_request, post_request,
    },
    model::{
        catalog::{CatalogRequest, RootCatalog},
        metadata::{ProtocolVersion, VersionResponse},
    },
    shared::{DATA_SERVICE, VERSION_ENDPOINT_PATH},
    store::Store,
};

pub(crate) async fn catalog_sync<T: Store>(token: CancellationToken, state: AppStateCatalog<T>) {
    for (name, connector) in state.federation.as_ref() {
        let token = token.clone();
        tokio::spawn(perform_sync(
            token,
            state.clone(),
            name.clone(),
            connector.clone(),
        ));
    }
}

async fn perform_sync<T: Store>(
    token: CancellationToken,
    state: AppStateCatalog<T>,
    name: String,
    connector: RemoteConnector,
) {
    let sync_interval_normal: Duration = Duration::from_secs(connector.sync_interval_secs);
    let sync_interval_escalated: Duration = Duration::from_secs(30);

    let mut ticker = interval(sync_interval_normal);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    let mut selected: Option<(String, ProtocolVersion)> = None;
    loop {
        select! {
            _ = token.cancelled() => {
                return
            }
            _ = ticker.tick() => {
                if do_sync(&state, &name, &connector, &mut selected).await {
                    ticker.reset_after(sync_interval_normal);
                } else {
                    ticker.reset_after(sync_interval_escalated);
                }
            }
        }
    }
}

async fn do_sync<T: Store>(
    state: &AppStateCatalog<T>,
    name: &str,
    connector: &RemoteConnector,
    selected: &mut Option<(String, ProtocolVersion)>,
) -> bool {
    let root = match discover_dsp_root(state, &connector.did).await {
        Ok(root) => root,
        Err(err) => {
            error!("Failed to discover the DSP endpoint for connector {name}, error: {err}");
            return false;
        }
    };

    // A peer that moved its endpoint invalidates the version picked against the old one.
    if selected
        .as_ref()
        .is_some_and(|(cached_root, _)| *cached_root != root)
    {
        info!("Connector {name} moved to {root}, re-selecting version");
        selected.take();
    }

    if selected.is_none() {
        match select_version(state, &root).await {
            Ok(Some(v)) => {
                info!("Using version {} for connector {name}", v.version);
                selected.replace((root.clone(), v));
            }
            Ok(_) => {
                warn!("No supported version for connector {name}");
                return false;
            }
            Err(err) => {
                error!("Failed to select version for connector {name}, error: {err}");
                return false;
            }
        }
    }
    let Some((_, version)) = selected.as_ref() else {
        return false;
    };

    let access_token = match state
        .authenticator
        .get_token(&state.client, &root, connector.did.clone())
        .await
    {
        Ok(t) => {
            debug!("Successfully retrieved access token for connector {name}");
            t
        }
        Err(err) => {
            error!("Failed to retrieve an access token for connector {name}, error: {err}");
            return false;
        }
    };

    match fetch_and_sync_catalog(state, name, &root, access_token.clone(), version).await {
        Ok(_) => true,
        Err(err) => {
            error!("Failed to perform catalog sync for connector {name}, error: {err}");
            false
        }
    }
}

/// The peer's `<root>`, read from the `DataService` entry in its DID document. That entry
/// names the version endpoint, which by construction sits directly under the root.
async fn discover_dsp_root<T: Store>(
    state: &AppStateCatalog<T>,
    did: &str,
) -> anyhow::Result<String> {
    let document = state.authenticator.resolve_peer(did).await?;

    let endpoint = document
        .service_endpoint(DATA_SERVICE)
        .with_context(|| format!("{did} advertises no {DATA_SERVICE}"))?;

    dsp_root_from_version_endpoint(endpoint)
}

fn dsp_root_from_version_endpoint(endpoint: &str) -> anyhow::Result<String> {
    endpoint
        .strip_suffix(VERSION_ENDPOINT_PATH)
        .map(str::to_string)
        .with_context(|| {
            format!("{DATA_SERVICE} endpoint {endpoint} does not end in {VERSION_ENDPOINT_PATH}")
        })
}

async fn select_version<T: Store>(
    state: &AppStateCatalog<T>,
    remote_address: &str,
) -> anyhow::Result<Option<ProtocolVersion>> {
    let versions: VersionResponse = get_request(
        &state.client,
        &state.validator,
        &format!("{remote_address}{VERSION_ENDPOINT_PATH}"),
        None,
    )
    .await?;

    // TODO: define our own authentication protocol and check whether the version supports it

    Ok(versions
        .protocol_versions
        .into_iter()
        .filter(|v| v.version == "2025-1" && v.binding == "HTTPS")
        .next())
}

async fn fetch_and_sync_catalog<T: Store>(
    state: &AppStateCatalog<T>,
    name: &str,
    remote_address: &str,
    access_token: String,
    version: &ProtocolVersion,
) -> anyhow::Result<()> {
    let request = CatalogRequest::new(None);
    let root_catalog = post_request(
        &state.client,
        &state.validator,
        &format!(
            "{remote_address}{path}/catalog/request",
            path = version.path
        ),
        Some(access_token),
        &request,
        |_, response: Option<RootCatalog>| response.unwrap(),
    )
    .await?;

    sync_catalog(state, name, root_catalog).await?;

    Ok(())
}

async fn sync_catalog<T: Store>(
    state: &AppStateCatalog<T>,
    name: &str,
    catalog: RootCatalog,
) -> anyhow::Result<()> {
    let datasets = catalog.extract_datasets();

    for dataset in &datasets {
        match state.store.save_federated_dataset(name, dataset).await {
            Ok(_) => info!(
                "Sucessfully stored federated dataset {} for connector {name}",
                dataset.dataset.resource.id
            ),
            Err(err) => error!(
                "Failed to store federated dataset {} for connector {name}, error: {err}",
                dataset.dataset.resource.id
            ),
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dsp_root_from_version_endpoint() {
        assert_eq!(
            dsp_root_from_version_endpoint(
                "https://dsp.partner.example.com/.well-known/dspace-version"
            )
            .expect("root derives"),
            "https://dsp.partner.example.com"
        );

        // The DSP root may sit under a path, and need not share a host with the DID.
        assert_eq!(
            dsp_root_from_version_endpoint(
                "https://partner.example.com/connector/.well-known/dspace-version"
            )
            .expect("root derives"),
            "https://partner.example.com/connector"
        );

        // `did-service-schema.json` constrains every entry to end in `/catalog`, which
        // for a DataService contradicts the spec's own example. Reject it rather than
        // guess at a root.
        dsp_root_from_version_endpoint("https://partner.example.com/catalog")
            .expect_err("a non-version endpoint must be rejected");
    }
}
