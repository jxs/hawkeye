use crate::{config, templates};
use hawkeye_core::models::Status;
use k8s_openapi::api::apps::v1::Deployment;
use kube::api::{Patch, PatchParams};
use kube::Api;
use serde::Deserialize;
use serde_json::json;
use thiserror::Error;

const FIELD_MGR: &str = "hawkeye_api";

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

/// Get a Watcher's Kubernetes deployment.
async fn get_watcher_deployment(
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
async fn scale_watcher_deployment(k8s_client: &kube::Client, watcher_id: &str, replica_count: u16) {
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
        .unwrap();
}

/// Update a Watcher's status to indicate it should be running or not.
async fn update_watcher_deployment_target_status(
    k8s_client: &kube::Client,
    watcher_id: &str,
    status: Status,
) {
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
        .unwrap();
}

pub async fn start_watcher(k8s_client: &kube::Client, watcher_id: &str) -> WatcherStartStatus {
    let deployment = match get_watcher_deployment(k8s_client, watcher_id).await {
        Ok(d) => d,
        _ => return WatcherStartStatus::NotFound,
    };

    // Actions and guards based on the current Watcher status.
    match get_watcher_status(&deployment) {
        Status::Running => WatcherStartStatus::AlreadyRunning,
        Status::Pending => WatcherStartStatus::CurrentlyUpdating,
        Status::Error => WatcherStartStatus::InErrorState,
        Status::Ready => {
            scale_watcher_deployment(k8s_client, watcher_id, 1_u16).await;
            update_watcher_deployment_target_status(k8s_client, watcher_id, Status::Running).await;
            WatcherStartStatus::Starting
        }
    }
}

pub async fn stop_watcher(k8s_client: &kube::Client, watcher_id: &str) -> WatcherStopStatus {
    let deployment = match get_watcher_deployment(k8s_client, watcher_id).await {
        Ok(d) => d,
        _ => return WatcherStopStatus::NotFound,
    };

    match get_watcher_status(&deployment) {
        Status::Ready => WatcherStopStatus::AlreadyStopped,
        Status::Pending => WatcherStopStatus::CurrentlyUpdating,
        Status::Error => WatcherStopStatus::InErrorState,
        Status::Running => {
            scale_watcher_deployment(k8s_client, watcher_id, 0_u16).await;
            update_watcher_deployment_target_status(k8s_client, watcher_id, Status::Ready).await;
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
