use crate::handlers::WatcherStatus;
use crate::{config, templates};
use hawkeye_core::models::{Status, Watcher};
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::autoscaling::v1::Scale;
use k8s_openapi::api::core::v1::{ConfigMap, Service};
use kube::api::{Patch, PatchParams};
use kube::Api;
use serde::Deserialize;
use serde_json::json;
use std::time;

const FIELD_MGR: &str = "hawkeye_api";

#[derive(Debug, Deserialize, thiserror::Error)]
pub enum WatcherStartStatus {
    #[error("Watcher is already running.")]
    AlreadyRunning, // OK
    #[error("Watcher is updating so it cannot be started.")]
    CurrentlyUpdating, // CONFLICT
    #[error("Watcher is starting.")]
    Starting, // OK
    #[error("Watcher is in an error state and cannot be stopped.")]
    InErrorState, // NOT_ACCEPTABLE
    #[error("Watcher not found.")]
    NotFound, // 404
    #[error("Watcher encountered an internal error.")]
    InternalError, // INTERNAL_ERROR
}

#[derive(Debug, Deserialize, thiserror::Error)]
pub enum WatcherStopStatus {
    #[error("Watcher is already stopped.")]
    AlreadyStopped, // OK
    #[error("Watcher is updating so it cannot be stopped.")]
    CurrentlyUpdating, // conflict
    #[error("Watcher is stopping.")]
    Stopping, // ok
    #[error("Watcher is in an error state and cannot be stopped.")]
    InErrorState, // NOT_ACCEPTABLE
    #[error("Watcher not found.")]
    NotFound, // 404
    #[error("Watcher encountered an internal error.")]
    InternalError, // INTERNAL_ERROR
}

/// Get a Watcher's Kubernetes ConfigMap. It represents a source of truth for Watcher config.
pub async fn get_watcher_configmap(
    k8s_client: &kube::Client,
    watcher_id: &str,
) -> kube::Result<ConfigMap> {
    let config_maps_client: Api<ConfigMap> =
        Api::namespaced(k8s_client.clone(), &config::NAMESPACE);
    config_maps_client
        .get(&templates::configmap_name(watcher_id))
        .await
}

/// Get a Watcher's Kubernetes deployment.
pub async fn get_watcher_deployment(
    k8s_client: &kube::Client,
    watcher_id: &str,
) -> kube::Result<Deployment> {
    let deployments_client: Api<Deployment> =
        Api::namespaced(k8s_client.clone(), &config::NAMESPACE);
    deployments_client
        .get(&templates::deployment_name(watcher_id))
        .await
}

/// Scale a Watcher's Kubernetes deployment by altering the number of replicas. Great for turning down to 0.
pub async fn scale_watcher_deployment(
    k8s_client: &kube::Client,
    watcher_id: &str,
    replica_count: u16,
) -> kube::Result<Scale> {
    let patch_params = PatchParams {
        field_manager: Some(FIELD_MGR.to_owned()),
        ..Default::default()
    };
    let deployment_scale_json = json!({
        "apiVersion": "autoscaling/v1",
        "spec": {"replicas": replica_count},
    });
    let deployments_client: Api<Deployment> =
        Api::namespaced(k8s_client.clone(), &config::NAMESPACE);
    deployments_client
        .patch_scale(
            &templates::deployment_name(watcher_id),
            &patch_params,
            &Patch::Merge(&deployment_scale_json),
        )
        .await
}

/// Update a Watcher's status to indicate it should be running or not.
pub async fn update_watcher_deployment_target_status(
    k8s_client: &kube::Client,
    watcher_id: &str,
    status: Status,
) -> kube::Result<Deployment> {
    let patch_params = PatchParams {
        field_manager: Some(FIELD_MGR.to_owned()),
        ..Default::default()
    };
    let status_label_json = json!({
        "apiVersion": "apps/v1",
        "metadata": {
            "labels": {
                "target_status": status,
            }
        }
    });
    let deployments_client: Api<Deployment> =
        Api::namespaced(k8s_client.clone(), &config::NAMESPACE);
    deployments_client
        .patch(
            &templates::deployment_name(watcher_id),
            &patch_params,
            &Patch::Merge(status_label_json),
        )
        .await
}

/// Start a Watcher by setting its replica count to 1.
pub async fn start_watcher(
    k8s_client: &kube::Client,
    watcher_id: &str,
) -> Result<WatcherStartStatus, kube::Error> {
    log::debug!("Starting Watcher {}", watcher_id);
    let deployment = get_watcher_deployment(k8s_client, watcher_id).await?;

    // Actions and guards based on the current Watcher status.
    let status = match deployment.get_watcher_status() {
        Status::Running => WatcherStartStatus::AlreadyRunning,
        Status::Pending => WatcherStartStatus::CurrentlyUpdating,
        Status::Error => WatcherStartStatus::InErrorState,
        Status::Ready => {
            scale_watcher_deployment(k8s_client, watcher_id, 1_u16).await?;
            update_watcher_deployment_target_status(k8s_client, watcher_id, Status::Running)
                .await?;
            WatcherStartStatus::Starting
        }
    };
    log::debug!(
        "Starting Watcher {} resulted in status: {:?}",
        watcher_id,
        status
    );

    Ok(status)
}

/// Stop a Watcher by setting its replica count to 0.
pub async fn stop_watcher(
    k8s_client: &kube::Client,
    watcher_id: &str,
) -> Result<WatcherStopStatus, kube::Error> {
    log::debug!("Stopping Watcher {}", watcher_id);
    let deployment = get_watcher_deployment(k8s_client, watcher_id).await?;
    let status = match deployment.get_watcher_status() {
        Status::Ready => WatcherStopStatus::AlreadyStopped,
        Status::Pending => WatcherStopStatus::CurrentlyUpdating,
        Status::Error => WatcherStopStatus::InErrorState,
        Status::Running => {
            scale_watcher_deployment(k8s_client, watcher_id, 0_u16).await?;
            update_watcher_deployment_target_status(k8s_client, watcher_id, Status::Ready).await?;
            WatcherStopStatus::Stopping
        }
    };
    log::debug!(
        "Stopping Watcher {} resulted in status: {:?}",
        watcher_id,
        status
    );

    Ok(status)
}

pub fn get_watcher_status(deployment: &Deployment) -> Status {
    let target_status = deployment
        .metadata
        .labels
        .as_ref()
        .and_then(|labels| {
            labels.get("target_status").map(|status| {
                let s: Status = serde_json::from_str(&format!("\"{}\"", status.to_lowercase()))
                    .unwrap_or(Status::Error);
                s
            })
        })
        .unwrap_or_else(|| {
            let name = deployment
                .metadata
                .name
                .as_ref()
                .expect("Name must be present");
            log::error!(
                "Deployment {} is missing required 'target_status' label",
                name
            );
            Status::Error
        });

    if let Some(status) = deployment.status.as_ref() {
        let deploy_status = if status.available_replicas.unwrap_or(0) > 0 {
            Status::Running
        } else {
            Status::Ready
        };
        log::debug!(
            "backend::get_watcher_status deploy_status={} target_status={}",
            deploy_status,
            target_status
        );
        match (deploy_status, target_status) {
            (Status::Running, Status::Running) => Status::Running,
            (Status::Ready, Status::Ready) => Status::Ready,
            (Status::Ready, Status::Running) => Status::Pending,
            (Status::Running, Status::Ready) => Status::Pending,
            (_, _) => Status::Error,
        }
    } else {
        Status::Error
    }
}

/// Update a Watcher's Kubernetes ConfigMap.
pub async fn update_watcher_configmap(
    k8s_client: &kube::Client,
    watcher: &Watcher,
) -> kube::Result<ConfigMap> {
    log::debug!("Updating ConfigMap instance");
    let config_file_contents = serde_json::to_string(&watcher).unwrap();
    let config = templates::build_configmap(
        watcher.id.as_ref().unwrap(),
        &config_file_contents,
        watcher.tags.as_ref(),
    );
    let patch_params = PatchParams {
        field_manager: Some(FIELD_MGR.to_owned()),
        ..Default::default()
    };
    let patch = Patch::Merge(&config);
    let config_maps: Api<ConfigMap> = Api::namespaced(k8s_client.clone(), &config::NAMESPACE);
    config_maps
        .patch(
            &templates::configmap_name(watcher.id.as_ref().unwrap()),
            &patch_params,
            &patch,
        )
        .await
}

pub async fn update_watcher_deployment(
    k8s_client: &kube::Client,
    watcher: &Watcher,
) -> kube::Result<Deployment> {
    // 2. Update Deployment with replicas=0
    log::debug!("Updating Deployment instance");
    let deploy = templates::build_deployment(
        watcher.id.as_ref().unwrap(),
        watcher.source.ingest_port,
        watcher.tags.as_ref(),
    );
    let patch_params = PatchParams {
        field_manager: Some(FIELD_MGR.to_owned()),
        ..Default::default()
    };
    let patch = Patch::Merge(&deploy);
    let deployments: Api<Deployment> = Api::namespaced(k8s_client.clone(), &config::NAMESPACE);
    deployments
        .patch(
            &templates::deployment_name(watcher.id.as_ref().unwrap()),
            &patch_params,
            &patch,
        )
        .await
}

pub async fn update_watcher_service(
    k8s_client: &kube::Client,
    watcher: &Watcher,
) -> kube::Result<Service> {
    log::debug!("Updating Service instance");
    let svc = templates::build_service(
        watcher.id.as_ref().unwrap(),
        watcher.source.ingest_port,
        watcher.tags.as_ref(),
    );
    // TODO: Handle errors
    let patch_params = PatchParams {
        field_manager: Some(FIELD_MGR.to_owned()),
        ..Default::default()
    };
    let patch = Patch::Merge(&svc);
    let services: Api<Service> = Api::namespaced(k8s_client.clone(), &config::NAMESPACE);
    services
        .patch(
            &templates::service_name(watcher.id.as_ref().unwrap()),
            &patch_params,
            &patch,
        )
        .await
}

// Apply Watcher updates and restart Watcher if it was previously running.
pub async fn apply_watcher_updates(
    k8s_client: &kube::Client,
    watcher: &Watcher,
) -> Result<(), kube::Error> {
    // Update k8s resources.
    let _ = update_watcher_configmap(k8s_client, watcher).await?;
    let _ = update_watcher_deployment(k8s_client, watcher).await?;
    let _ = update_watcher_service(k8s_client, watcher).await?;

    // Only restart the worker if it was already started.
    let retry_sleep = time::Duration::from_secs(1);
    log::warn!("Watcher status: {:?}", watcher.status);
    if watcher.status != Some(Status::Ready) {
        let watcher_id = watcher.id.as_ref().unwrap();
        let _ = stop_watcher(k8s_client, watcher_id).await?;

        // k8s can take a few seconds to update, so retry a few times.
        for _ in 1..8 {
            let deployment_status = get_watcher_deployment(k8s_client, watcher_id).await?;
            // .map_err(|_| warp::reject::custom(InternalError));
            if deployment_status.get_watcher_status() == Status::Ready {
                break;
            }
            let _ = tokio::time::sleep(retry_sleep);
        }

        let _ = start_watcher(k8s_client, watcher_id).await?;
    }

    Ok(())
}
