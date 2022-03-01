use crate::{config, templates};
use hawkeye_core::models::Status;
use k8s_openapi::api::apps::v1::Deployment;
use kube::api::{Patch, PatchParams};
use kube::Api;
use serde::Deserialize;
use serde_json::json;
use thiserror::Error;

#[derive(Debug, Deserialize, Error)]
pub enum WatcherStartStatus {
    #[error("Watcher is already running.")]
    AlreadyRunning, // ok
    #[error("Watcher is updating so it cannot be started.")]
    CurrentlyUpdating, // conflict
    #[error("Watcher is starting.")]
    Starting, // ok
    #[error("Watcher is in an error state and cannot be stopped.")]
    InErrorState, // NOT_ACCEPTABLE
    #[error("Watcher not found.")]
    NotFound, // 404
}

#[derive(Debug, Deserialize, Error)]
pub enum WatcherStopStatus {
    #[error("Watcher is already stopped.")]
    AlreadyStopped, // ok
    #[error("Watcher is updating so it cannot be stopped.")]
    CurrentlyUpdating, // conflict
    #[error("Watching is stopping.")]
    Stopping, // ok
    #[error("Watcher is in an error state and cannot be stopped.")]
    InErrorState, // NOT_ACCEPTABLE
    #[error("Watcher not found.")]
    NotFound, // 404
}

pub async fn start_watcher(k8s_client: &kube::Client, watcher_id: &str) -> WatcherStartStatus {
    let deployments_client: Api<Deployment> =
        Api::namespaced(k8s_client.clone(), &config::NAMESPACE);
    let deployment = match deployments_client
        .get(&templates::deployment_name(watcher_id))
        .await
    {
        Ok(d) => d,
        Err(_) => return WatcherStartStatus::NotFound,
    };

    // Actions and guards based on the current Watcher status.
    match get_watcher_status(&deployment) {
        Status::Running => WatcherStartStatus::AlreadyRunning,
        Status::Pending => WatcherStartStatus::CurrentlyUpdating,
        Status::Error => WatcherStartStatus::InErrorState,
        Status::Ready => {
            // Start Watcher by setting Kubernetes deployment replicas=1
            let patch_params = PatchParams {
                field_manager: Some("hawkeye_api".to_string()),
                ..Default::default()
            };

            // Set Kubernetes deployment replica=1 via patch.
            let deployment_scale_json = json!({
                "apiVersion": "autoscaling/v1",
                "spec": { "replicas": 1_u16 },
            });
            deployments_client
                .patch_scale(
                    deployment.metadata.name.as_ref().unwrap(),
                    &patch_params,
                    &Patch::Merge(&deployment_scale_json),
                )
                .await
                .unwrap();

            // Update the status of the Watcher to indicate it should be running.
            let status_label_json = json!({
                "apiVersion": "apps/v1",
                "metadata": {
                    "labels": {
                        "target_status": Status::Running,
                    }
                }
            });
            deployments_client
                .patch(
                    deployment.metadata.name.as_ref().unwrap(),
                    &patch_params,
                    &Patch::Merge(status_label_json),
                )
                .await
                .unwrap();

            WatcherStartStatus::Starting
        }
    }
}

pub async fn stop_watcher(k8s_client: &kube::Client, watcher_id: &str) -> WatcherStopStatus {
    let deployments_client: Api<Deployment> =
        Api::namespaced(k8s_client.clone(), &config::NAMESPACE);
    // TODO: probably better to just get the scale
    let deployment = match deployments_client
        .get(&templates::deployment_name(watcher_id))
        .await
    {
        Ok(d) => d,
        Err(_) => return WatcherStopStatus::NotFound,
    };

    match get_watcher_status(&deployment) {
        Status::Ready => WatcherStopStatus::AlreadyStopped,
        Status::Pending => WatcherStopStatus::CurrentlyUpdating,
        Status::Error => WatcherStopStatus::InErrorState,
        Status::Running => {
            // Stop watcher / replicas to 0
            let patch_params = PatchParams {
                field_manager: Some("hawkeye_api".to_string()),
                ..Default::default()
            };

            let deployment_scale_json = json!({
                "apiVersion": "autoscaling/v1",
                "spec": { "replicas": 0_u16 },
            });
            deployments_client
                .patch_scale(
                    deployment.metadata.name.as_ref().unwrap(),
                    &patch_params,
                    &Patch::Merge(&deployment_scale_json),
                )
                .await
                .unwrap();

            // Update the status of the Watcher to indicate it should be running.
            let status_label_json = json!({
                "apiVersion": "apps/v1",
                "metadata": {
                    "labels": {
                        "target_status": Status::Ready,
                    }
                }
            });
            deployments_client
                .patch(
                    deployment.metadata.name.as_ref().unwrap(),
                    &patch_params,
                    &Patch::Merge(status_label_json),
                )
                .await
                .unwrap();

            WatcherStopStatus::Stopping
        }
    }
}

fn get_watcher_status(deployment: &Deployment) -> Status {
    let target_status = deployment
        .metadata
        .labels
        .as_ref()
        .and_then(|labels| {
            labels
                .get("target_status")
                .map(|status| serde_json::from_str(&format!("\"{}\"", status)).ok())
        })
        .flatten()
        .flatten()
        .unwrap_or({
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
