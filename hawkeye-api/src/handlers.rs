use crate::backend::{WatcherStartStatus, WatcherStopStatus};
use crate::config::{CALL_WATCHER_TIMEOUT, NAMESPACE};
use crate::filters::ErrorResponse;
use crate::templates::container_spec;
use crate::{backend, config, templates};
use hawkeye_core::models::{Status, Watcher};
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::{ConfigMap, Pod, Service};
use kube::api::{DeleteParams, ListParams, Patch, PatchParams, PostParams};
use kube::{Api, Client};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::convert::Infallible;
use std::time::Duration;
use uuid::Uuid;
use warp::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use warp::http::{HeaderValue, StatusCode};
use warp::hyper::Body;
use warp::reply;

pub async fn list_watchers(client: Client) -> Result<impl warp::Reply, Infallible> {
    let lp = ListParams::default()
        .labels("app=hawkeye,watcher_id")
        .timeout(10);

    // Get all K8S deployments we know, we want to return the status of each watcher
    let deployments_client: Api<Deployment> = Api::namespaced(client.clone(), &NAMESPACE);
    let deployments = deployments_client.list(&lp).await.unwrap();
    let mut deployments_index = HashMap::new();
    for deploy in deployments.items {
        if let Some(watcher_id) = deploy.metadata.labels.as_ref().unwrap().get("watcher_id") {
            deployments_index.insert(watcher_id.clone(), deploy.get_watcher_status());
        }
    }

    let config_maps_client: Api<ConfigMap> = Api::namespaced(client.clone(), &NAMESPACE);
    let config_maps = config_maps_client.list(&lp).await.unwrap();

    let mut watchers: Vec<Watcher> = Vec::new();
    for config in config_maps.items {
        let data = config.data.unwrap();
        let mut watcher: Watcher = serde_json::from_str(data.get("watcher.json").unwrap()).unwrap();
        let calculated_status = if let Some(status) =
            deployments_index.get(watcher.id.as_ref().unwrap_or(&"undefined".to_string()))
        {
            *status
        } else {
            Status::Error
        };
        watcher.status = Some(calculated_status);
        // TODO: Comes from the service
        watcher.source.ingest_ip = None;
        watchers.push(watcher);
    }

    Ok(warp::reply::json(&watchers))
}

pub async fn create_watcher(
    mut watcher: Watcher,
    client: Client,
) -> Result<impl warp::Reply, Infallible> {
    log::debug!("v1.create_watcher: {:?}", watcher);

    // TODO: THis could use another iteration to make it happen more automatically.
    if let Some(err) = watcher.is_valid().err() {
        let fe: ErrorResponse = err.into();
        return Ok(reply::with_status(
            reply::json(&fe),
            StatusCode::UNPROCESSABLE_ENTITY,
        ));
    }

    let new_id = Uuid::new_v4().to_string();
    watcher.id = Some(new_id.clone());
    let pp = PostParams::default();

    // 1. Create ConfigMap
    log::debug!("Creating ConfigMap instance");
    let config_maps: Api<ConfigMap> = Api::namespaced(client.clone(), &NAMESPACE);
    let config_file_contents = serde_json::to_string(&watcher).unwrap();
    let config = templates::build_configmap(&new_id, &config_file_contents, watcher.tags.as_ref());
    // TODO: Handle errors
    let _ = config_maps.create(&pp, &config).await.unwrap();

    // 2. Create Deployment with replicas=0
    log::debug!("Creating Deployment instance");
    let deployments: Api<Deployment> = Api::namespaced(client.clone(), &NAMESPACE);
    let deploy =
        templates::build_deployment(&new_id, watcher.source.ingest_port, watcher.tags.as_ref());
    // TODO: Handle errors
    let _ = deployments.create(&pp, &deploy).await.unwrap();

    // 3. Create Service/LoadBalancer
    log::debug!("Creating Service instance");
    let services: Api<Service> = Api::namespaced(client.clone(), &NAMESPACE);
    let svc = templates::build_service(&new_id, watcher.source.ingest_port, watcher.tags.as_ref());
    // TODO: Handle errors
    let _ = services.create(&pp, &svc).await.unwrap();

    watcher.status = Some(Status::Pending);
    watcher.source.ingest_ip = None;

    Ok(reply::with_status(
        reply::json(&watcher),
        StatusCode::CREATED,
    ))
}

pub async fn update_watcher(
    watcher_id: String,
    payload_watcher: Watcher,
    k8s_client: Client,
) -> Result<impl warp::Reply, Infallible> {
    log::debug!("v1.update_watcher: {:?}", payload_watcher);

    if let Some(err) = payload_watcher.is_valid().err() {
        let fe: ErrorResponse = err.into();
        return Ok(reply::with_status(
            reply::json(&fe),
            StatusCode::UNPROCESSABLE_ENTITY,
        ));
    }

    let config_map = match backend::get_watcher_configmap(&k8s_client, &watcher_id).await {
        Ok(cm) => cm,
        Err(_) => {
            return Ok(reply::with_status(
                reply::json(&json!({})),
                StatusCode::NOT_FOUND,
            ))
        }
    };

    let deployment = match backend::get_watcher_deployment(&k8s_client, &watcher_id).await {
        Ok(d) => d,
        Err(_) => {
            log::error!(
                "A ConfigMap was found, but the Deployment was missing for Watcher {watcher_id}. Odd."
            );
            return Ok(reply::with_status(
                reply::json(&json!({})),
                StatusCode::NOT_FOUND,
            ));
        }
    };

    let mut existing_watcher: Watcher =
        serde_json::from_str(config_map.data.unwrap().get("watcher.json").unwrap()).unwrap();
    existing_watcher.merge(payload_watcher);

    let (_, _, _) = tokio::join!(
        backend::update_watcher_configmap(&k8s_client, &existing_watcher),
        backend::update_watcher_deployment(&k8s_client, &existing_watcher),
        backend::update_watcher_service(&k8s_client, &existing_watcher),
    );

    backend::stop_watcher(&k8s_client, existing_watcher.id.as_ref().unwrap()).await;
    backend::start_watcher(&k8s_client, existing_watcher.id.as_ref().unwrap()).await;
    existing_watcher.status = Some(deployment.get_watcher_status());

    Ok(reply::with_status(
        reply::json(&existing_watcher),
        StatusCode::OK,
    ))
}

pub async fn upgrade_watcher(
    watcher_id: String,
    client: Client,
) -> Result<impl warp::Reply, Infallible> {
    log::debug!("v1.upgrade_watcher: {}", watcher_id);
    let deployments: Api<Deployment> = Api::namespaced(client.clone(), &NAMESPACE);
    let deployment = match deployments
        .get(&templates::deployment_name(&watcher_id))
        .await
    {
        Ok(d) => d,
        Err(_) => {
            return Ok(reply::with_status(
                reply::json(&json!({})),
                StatusCode::NOT_FOUND,
            ))
        }
    };

    // We use the ConfigMap as source of truth for what are the watchers we have
    let config_maps_client: Api<ConfigMap> = Api::namespaced(client.clone(), &NAMESPACE);
    let config_map = match config_maps_client
        .get(&templates::configmap_name(&watcher_id))
        .await
    {
        Ok(c) => c,
        Err(_) => {
            return Ok(reply::with_status(
                reply::json(&json!({})),
                StatusCode::NOT_FOUND,
            ))
        }
    };

    let mut watcher: Watcher =
        serde_json::from_str(config_map.data.unwrap().get("watcher.json").unwrap()).unwrap();
    let watcher_status = deployment.get_watcher_status();
    if watcher_status != Status::Ready {
        return Ok(reply::with_status(
            reply::json(
                &json!({"message": "The Watcher must be stopped before the upgrade can be applied"}),
            ),
            StatusCode::BAD_REQUEST,
        ));
    }
    watcher.status = Some(watcher_status);

    let patch_params = PatchParams::default();
    let spec_updated = json!({
        "spec": {
            "template": {
                "spec": {
                    "containers": [
                        container_spec(&watcher_id, watcher.source.ingest_port)
                    ]
                }
            }
        }
    });

    match deployments
        .patch(
            deployment.metadata.name.as_ref().unwrap(),
            &patch_params,
            &Patch::Apply(spec_updated),
        )
        .await
    {
        Ok(_) => Ok(reply::with_status(reply::json(&watcher), StatusCode::OK)),
        Err(e) => {
            let msg: String = format!("Error while calling Kubernetes API: {:?}", e);
            log::error!("{}", msg);
            let error_body = json!({ "message": msg });
            Ok(reply::with_status(
                reply::json(&error_body),
                StatusCode::INTERNAL_SERVER_ERROR,
            ))
        }
    }
}

pub async fn get_watcher(
    watcher_id: String,
    client: Client,
) -> Result<impl warp::Reply, Infallible> {
    let deployments_client: Api<Deployment> = Api::namespaced(client.clone(), &NAMESPACE);
    // TODO: searching for a deployment could be a filter in this route
    let deployment = match deployments_client
        .get(&templates::deployment_name(&watcher_id))
        .await
    {
        Ok(d) => d,
        Err(_) => {
            return Ok(reply::with_status(
                reply::json(&json!({})),
                StatusCode::NOT_FOUND,
            ))
        }
    };

    // We use the ConfigMap as source of truth for what are the watchers we have
    let config_maps_client: Api<ConfigMap> = Api::namespaced(client.clone(), &NAMESPACE);
    let config_map = match config_maps_client
        .get(&templates::configmap_name(&watcher_id))
        .await
    {
        Ok(c) => c,
        Err(_) => {
            return Ok(reply::with_status(
                reply::json(&json!({})),
                StatusCode::NOT_FOUND,
            ))
        }
    };

    let mut w: Watcher =
        serde_json::from_str(config_map.data.unwrap().get("watcher.json").unwrap()).unwrap();
    w.status = Some(deployment.get_watcher_status());

    w.status_description = if let Some(Status::Pending) = w.status.as_ref() {
        // Load more information why it's in pending status
        // We get the reason the container is waiting, if available
        let pods_client: Api<Pod> = Api::namespaced(client.clone(), &NAMESPACE);
        let lp = ListParams::default().labels(&format!("app=hawkeye,watcher_id={}", watcher_id));
        let pods = pods_client.list(&lp).await.unwrap();
        let status_description = pods
            .items
            .first()
            .and_then(|p| p.status.as_ref())
            .and_then(|ps| ps.container_statuses.as_ref())
            .and_then(|css| css.first())
            .and_then(|cs| cs.state.as_ref())
            .and_then(|cs| cs.waiting.as_ref())
            .and_then(|csw| csw.message.clone());
        log::debug!(
            "Additional information for the Pending status: {:?}",
            status_description.as_ref()
        );
        status_description
    } else {
        None
    };

    // Comes from the service
    w.source.ingest_ip = if w.status != Some(Status::Error) {
        log::debug!("Getting ingest_ip from Service's LoadBalancer");
        let services: Api<Service> = Api::namespaced(client.clone(), &NAMESPACE);
        let service = services
            .get_status(&templates::service_name(&watcher_id))
            .await
            .unwrap();
        service
            .status
            .as_ref()
            .and_then(|s| s.load_balancer.as_ref())
            .and_then(|lbs| lbs.ingress.as_ref())
            .and_then(|lbs| lbs.first())
            .and_then(|lb| lb.clone().hostname.or(lb.clone().ip))
    } else {
        None
    };

    Ok(reply::with_status(reply::json(&w), StatusCode::OK))
}

pub async fn get_video_frame(
    watcher_id: String,
    client: Client,
) -> Result<impl warp::Reply, Infallible> {
    let mut resp = warp::reply::Response::new(Body::empty());

    // We use the ConfigMap as source of truth for what are the watchers we have
    let config_maps_client: Api<ConfigMap> = Api::namespaced(client.clone(), &NAMESPACE);
    let config_map = match config_maps_client
        .get(&templates::configmap_name(&watcher_id))
        .await
    {
        Ok(c) => c,
        Err(_) => {
            log::debug!(
                "ConfigMap object not found for this watcher: {}",
                watcher_id
            );
            *resp.status_mut() = StatusCode::NOT_FOUND;
            return Ok(resp);
        }
    };
    let watcher: Watcher =
        serde_json::from_str(config_map.data.unwrap().get("watcher.json").unwrap()).unwrap();

    let deployments_client: Api<Deployment> = Api::namespaced(client.clone(), &NAMESPACE);
    let deployment = match deployments_client
        .get(&templates::deployment_name(&watcher_id))
        .await
    {
        Ok(d) => d,
        Err(_) => {
            *resp.status_mut() = StatusCode::NOT_FOUND;
            return Ok(resp);
        }
    };
    if Status::Running != deployment.get_watcher_status() {
        log::debug!("Watcher is not running...");
        *resp.status_mut() = StatusCode::NOT_ACCEPTABLE;
        return Ok(resp);
    }
    let pods_client: Api<Pod> = Api::namespaced(client.clone(), &NAMESPACE);
    let lp = ListParams::default().labels(&format!("app=hawkeye,watcher_id={}", watcher_id));
    let pods = pods_client.list(&lp).await.unwrap();
    if let Some(pod_ip) = pods
        .items
        .first()
        .and_then(|p| p.status.as_ref())
        .and_then(|ps| ps.pod_ip.clone())
    {
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(*CALL_WATCHER_TIMEOUT))
            .build()
            .unwrap();
        // Try for new and old ports in pod
        for port in &[watcher.source.ingest_port, 3030] {
            let url = format!("http://{}:{}/latest_frame", pod_ip, port);

            log::info!("Calling Pod using url: {}", url);
            let response = match http_client.get(url.as_str()).send().await {
                Ok(r) => r,
                Err(error) => {
                    log::error!("Could not call {} endpoint: {:?}", url, error);
                    *resp.status_mut() = StatusCode::EXPECTATION_FAILED;
                    return Ok(resp);
                }
            };

            match response.error_for_status() {
                Ok(image_response) => {
                    let headers = resp.headers_mut();
                    headers.insert(CONTENT_TYPE, HeaderValue::from_static("image/png"));
                    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));

                    let image_bytes = image_response.bytes().await.unwrap();
                    *resp.body_mut() = Body::from(image_bytes.to_vec());

                    return Ok(resp);
                }
                Err(_) => {
                    continue;
                }
            }
        }
        log::error!("Error calling Pod using old and new urls");
        *resp.status_mut() = StatusCode::EXPECTATION_FAILED;
    } else {
        log::debug!("Not able to get Pod IP");
        *resp.status_mut() = StatusCode::EXPECTATION_FAILED;
    }
    Ok(resp)
}

/// Start a Watcher worker by making sure there's a positive replica count for the Kubernetes
/// deployment.
pub async fn start_watcher(
    watcher_id: String,
    k8s_client: kube::Client,
) -> Result<impl warp::Reply, Infallible> {
    let status = backend::start_watcher(&k8s_client, &watcher_id).await;
    let (msg, status_code) = match status {
        WatcherStartStatus::NotFound => ("Watcher can not be found.".to_owned(), StatusCode::OK),
        WatcherStartStatus::AlreadyRunning => {
            ("Watcher is already running".to_owned(), StatusCode::OK)
        }
        WatcherStartStatus::CurrentlyUpdating => (
            "Watcher is currently updating".to_owned(),
            StatusCode::CONFLICT,
        ),
        WatcherStartStatus::InErrorState => (
            "Watcher in error state cannot be set to running".to_owned(),
            StatusCode::NOT_ACCEPTABLE,
        ),
        _ => {
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
            let deployments_client: Api<Deployment> =
                Api::namespaced(k8s_client.clone(), &config::NAMESPACE);
            deployments_client
                .patch_scale(
                    &templates::deployment_name(&watcher_id),
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
                    &templates::deployment_name(&watcher_id),
                    &patch_params,
                    &Patch::Merge(status_label_json),
                )
                .await
                .unwrap();

            ("Watcher is starting".to_owned(), StatusCode::OK)
        }
    };

    Ok(reply::with_status(
        reply::json(&json!({ "message": msg })),
        status_code,
    ))
}

/// Stop a Watcher worker by making sure there's a replica count of 0 for the Kubernetes
/// deployment.
pub async fn stop_watcher(
    watcher_id: String,
    k8s_client: Client,
) -> Result<impl warp::Reply, Infallible> {
    let status = backend::stop_watcher(&k8s_client, &watcher_id).await;
    let (msg, status_code) = match status {
        WatcherStopStatus::NotFound => ("Watcher can not be found.".to_owned(), StatusCode::OK),
        WatcherStopStatus::AlreadyStopped => {
            ("Watcher is already stopped".to_owned(), StatusCode::OK)
        }
        WatcherStopStatus::CurrentlyUpdating => (
            "Watcher is currently updating".to_owned(),
            StatusCode::CONFLICT,
        ),
        WatcherStopStatus::InErrorState => (
            "Watcher in error state cannot be set to stopped".to_owned(),
            StatusCode::NOT_ACCEPTABLE,
        ),
        _ => {
            // Stop watcher / replicas to 0
            let patch_params = PatchParams {
                field_manager: Some("hawkeye_api".to_string()),
                ..Default::default()
            };

            let deployment_scale_json: Value = json!({
                "apiVersion": "autoscaling/v1",
                "spec": { "replicas": 0_i16 },
            });
            let deployments_client: Api<Deployment> =
                Api::namespaced(k8s_client.clone(), &config::NAMESPACE);
            deployments_client
                .patch_scale(
                    &templates::deployment_name(&watcher_id),
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
                    &templates::deployment_name(&watcher_id),
                    &patch_params,
                    &Patch::Merge(status_label_json),
                )
                .await
                .unwrap();

            ("Watcher is stopping.".to_owned(), StatusCode::OK)
        }
    };

    Ok(reply::with_status(
        reply::json(&json!({ "message": msg })),
        status_code,
    ))
}

pub async fn delete_watcher(
    watcher_id: String,
    client: Client,
) -> Result<impl warp::Reply, Infallible> {
    let dp = DeleteParams::default();

    let deployments_client: Api<Deployment> = Api::namespaced(client.clone(), &NAMESPACE);
    let _ = deployments_client
        .delete(&templates::deployment_name(&watcher_id), &dp)
        .await;

    let config_maps: Api<ConfigMap> = Api::namespaced(client.clone(), &NAMESPACE);
    let _ = config_maps
        .delete(&templates::configmap_name(&watcher_id), &dp)
        .await;

    let services: Api<Service> = Api::namespaced(client, &NAMESPACE);
    match services
        .delete(&templates::service_name(&watcher_id), &dp)
        .await
    {
        Ok(_) => Ok(reply::with_status(
            reply::json(&json!({
                "message": "Watcher has been deleted"
            })),
            StatusCode::OK,
        )),
        Err(_) => Ok(reply::with_status(
            reply::json(&json!({
                "message": "Watcher does not exist"
            })),
            StatusCode::NOT_FOUND,
        )),
    }
}

pub async fn healthcheck(client: Client) -> Result<impl warp::Reply, Infallible> {
    match client.apiserver_version().await {
        Ok(_info) => Ok(reply::with_status(
            reply::json(&json!({
                "message": "All good! 🎉",
            })),
            StatusCode::OK,
        )),
        Err(err) => {
            log::error!("Cannot communicate with K8s API: {:?}", err);
            Ok(reply::with_status(
                reply::json(&json!({
                    "message": "Not able to communicate with the Kubernetes API Server.",
                })),
                StatusCode::SERVICE_UNAVAILABLE,
            ))
        }
    }
}

pub trait WatcherStatus {
    fn get_watcher_status(&self) -> Status;
}

impl WatcherStatus for Deployment {
    fn get_watcher_status(&self) -> Status {
        let target_status = self
            .metadata
            .labels
            .as_ref()
            .and_then(|labels| {
                labels
                    .get("target_status")
                    .map(|status| serde_json::from_str(&format!("\"{}\"", status)).ok())
            })
            .flatten()
            .unwrap_or({
                let name = self.metadata.name.as_ref().expect("Name must be present");
                log::error!(
                    "Deployment {} is missing required 'target_status' label",
                    name
                );
                Status::Error
            });

        if let Some(status) = self.status.as_ref() {
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
}
