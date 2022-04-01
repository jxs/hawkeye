use crate::{backend, config};
use hawkeye_core::models::Watcher;
use k8s_openapi::api::core::v1::ConfigMap;
use kube::api::ListParams;
use kube::Api;
use regex::Regex;

/// Migrate all Watcher payloads to the new Transition format.
/// - removed toplevel `slate_url` element.
/// - for each transition, to/from were exploded from a string to an object.
///     - frametype: context/slate
///     - slate_context.url (when frametype=slate)
pub async fn migration_multislate(k8s_client: &kube::Client) {
    let log_prefix = "MIGRATION_MULTISLATE";
    let configmaps_client: Api<ConfigMap> = Api::namespaced(k8s_client.clone(), &config::NAMESPACE);
    let lp = ListParams::default();
    let config_maps = configmaps_client.list(&lp).await.unwrap();

    // let mut watchers: Vec<Watcher> = Vec::new();
    for configmap in config_maps.items {
        log::info!(
            "{} Investigating k8s ConfigMap: {:?}",
            log_prefix,
            configmap
        );
        let data = configmap.data.unwrap();
        let watcher_json = match data.get("watcher.json") {
            Some(v) => v,
            None => continue,
        };

        let re_slate_url = Regex::new("\"slate_url\":\"(.*?)\",").unwrap();

        let slate_url = match re_slate_url.captures(watcher_json) {
            Some(u) => u.get(1).map(|m| m.as_str()).unwrap(),
            None => {
                log::info!(
                    "{} ConfigMap has already been migrated! Skipping...",
                    log_prefix
                );
                continue;
            }
        };

        // remove top level `slate_url` since we're moving it to the to:slate state.
        let migrated_json = &re_slate_url.replace(watcher_json, "").to_string();

        // Migrate content:from and content:to transitions
        let re_from_content = Regex::new("\"from\":\"content\"").unwrap();
        let migrated_json = &re_from_content
            .replace(migrated_json, "\"from\":{\"frame_type\":\"content\"}")
            .to_string();
        let re_to_content = Regex::new("\"to\":\"content\"").unwrap();
        let migrated_json = &re_to_content
            .replace(migrated_json, "\"to\":{\"frame_type\":\"content\"}")
            .to_string();

        // Migrate from:slate transitions
        let re_from_slate = Regex::new("\"from\":\"slate\"").unwrap();
        let replace = format!(
            "\"from\":{{\"frame_type\":\"slate\",\"slate_context\":{{\"url\":\"{}\"}}}}",
            slate_url
        );
        let migrated_json = &re_from_slate.replace(migrated_json, replace).to_string();

        // Migrate to:slate transitions
        let re_to_slate = Regex::new("\"to\":\"slate\"").unwrap();
        let replace = format!(
            "\"to\":{{\"frame_type\":\"slate\",\"slate_context\":{{\"url\":\"{}\"}}}}",
            slate_url
        );
        let migrated_json = &re_to_slate.replace(migrated_json, replace).to_string();

        // PATCH the k8s ConfigMap with the new migrated JSON.
        let watcher: Watcher = serde_json::from_str(migrated_json).unwrap();
        let _ = backend::update_watcher_configmap(k8s_client, &watcher).await;
        log::info!("{} Updated ConfigMap: {}", log_prefix, configmap.metadata.name.unwrap());
    }
}
