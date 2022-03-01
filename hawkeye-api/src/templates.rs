use crate::config::DOCKER_IMAGE;
use hawkeye_core::models::Status;
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::{ConfigMap, Service};
use serde_json::json;
use std::collections::HashMap;

const K8S_LABEL_API_TAG_PREFIX: &str = "hawkeye.api.tags/";

/// Builds an idempotent name for the `ConfigMap` based on the `watcher_id`.
pub fn configmap_name(watcher_id: &str) -> String {
    format!("hawkeye-config-{}", watcher_id)
}

/// A helper function to concat system tags with API tags.
pub fn sys_api_tags_concat(
    system_tags: &HashMap<&str, &str>,
    api_tags: Option<&HashMap<String, String>>,
) -> HashMap<String, String> {
    let mut tags = HashMap::new();
    system_tags.iter().for_each(|(key, value)| {
        tags.insert(key.to_string(), value.to_string());
    });
    api_tags
        .unwrap_or(&HashMap::new())
        .iter()
        .for_each(|(key, value)| {
            let transformed_key = match key.as_ref() {
                "app" => "app".to_string(),
                _ => format!("{}{}", K8S_LABEL_API_TAG_PREFIX, key),
            };
            tags.insert(transformed_key, value.to_string());
        });

    tags
}

/// Builds a `ConfigMap` in the format expected to run the hawkeye-worker.
pub fn build_configmap(
    watcher_id: &str,
    contents: &str,
    api_tags: Option<&HashMap<String, String>>,
) -> ConfigMap {
    serde_json::from_value(json!({
        "apiVersion": "v1",
        "kind": "ConfigMap",
        "metadata": {
            "name": configmap_name(watcher_id),
            "labels": sys_api_tags_concat(
                &HashMap::from([
                    ("app", "hawkeye"),
                    ("watcher_id", watcher_id),
                ]),
                api_tags,
            ),
        },
        "data": {
            "log_level": "INFO",
            "watcher.json": contents,
        },
    }))
    .unwrap()
}

/// Builds an idempotent name for the `Deployment` based on the `watcher_id`.
pub fn deployment_name(watcher_id: &str) -> String {
    format!("hawkeye-deploy-{}", watcher_id)
}

/// Builds a `Deployment` configured to run the hawkeye-worker process.
pub fn build_deployment(
    watcher_id: &str,
    ingest_port: u32,
    api_tags: Option<&HashMap<String, String>>,
) -> Deployment {
    let metric_port_str = ingest_port.to_string();
    serde_json::from_value(json!({
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "metadata": {
            "name": deployment_name(watcher_id),
            "labels": sys_api_tags_concat(
                &HashMap::from([
                    ("app", "hawkeye"),
                    ("watcher_id", watcher_id),
                    ("target_status", Status::Ready.to_string().as_str()),
                ]),
                api_tags,
            ),
        },
        "spec": {
            "replicas": 0,
            "selector": {
                "matchLabels": {
                    "app": "hawkeye",
                    "watcher_id": watcher_id,
                }
            },
            "template": {
                "metadata": {
                    "annotations": {
                        "prometheus.io/port": metric_port_str,
                        "prometheus.io/scrape": "true",
                        "prometheus.io/path": "metrics",
                    },
                    "labels": sys_api_tags_concat(
                        &HashMap::from([
                            ("app", "hawkeye"),
                            ("watcher_id", watcher_id),
                            ("target_status", Status::Ready.to_string().as_str()),
                            ("prometheus.io/port", metric_port_str.as_str()),
                            ("prometheus.io/scrape", "true"),
                            ("prometheus.io/path", "metrics"),
                        ]),
                        api_tags,
                    ),
                },
                "spec": {
                    "dnsPolicy": "Default",
                    "restartPolicy": "Always",
                    "terminationGracePeriodSeconds": 5,
                    "containers": [
                        container_spec(watcher_id, ingest_port)
                    ],
                    "volumes": [
                        {
                            "name": "config",
                            "configMap": {
                                "name": configmap_name(watcher_id),
                                "items": [
                                    {
                                        "key": "watcher.json",
                                        "path": "watcher.json"
                                    }
                                ]
                            }
                        }
                    ]
                }
            }
        }
    }))
    .unwrap()
}

/// Returns a fragment of the container specification
pub fn container_spec(watcher_id: &str, ingest_port: u32) -> serde_json::Value {
    json!({
        "name": "hawkeye-app",
        "imagePullPolicy": "IfNotPresent",
        "image": DOCKER_IMAGE.as_str(),
        "args": [
            "/config/watcher.json"
        ],
        "env": [
            {
                "name": "RUST_LOG",
                "valueFrom": {
                    "configMapKeyRef": {
                        "name": configmap_name(watcher_id),
                        "key": "log_level"
                    }
                }
            }
        ],
        "resources": {
            "limits": {
                "cpu": "2000m",
                "memory": "100Mi"
            },
            "requests": {
                "cpu": "1150m",
                "memory": "50Mi"
            }
        },
        "ports": [
            {
                "containerPort": ingest_port,
                "protocol": "UDP"
            },
            {
                "containerPort": ingest_port,
                "protocol": "TCP"
            }
        ],
        "volumeMounts": [
            {
                "mountPath": "/config",
                "name": "config",
                "readOnly": true
            }
        ]
    })
}

/// Builds an idempotent name for the `Service` based on the `watcher_id`.
pub fn service_name(watcher_id: &str) -> String {
    format!("hawkeye-vid-svc-{}", watcher_id)
}

/// Builds a `Service` in the format expected to expose the hawkeye-worker.
pub fn build_service(
    watcher_id: &str,
    ingest_port: u32,
    api_tags: Option<&HashMap<String, String>>,
) -> Service {
    serde_json::from_value(json!({
        "apiVersion": "v1",
        "kind": "Service",
        "metadata": {
            "name": service_name(watcher_id),
            "labels": sys_api_tags_concat(
                &HashMap::from([
                    ("app", "hawkeye"),
                    ("watcher_id", watcher_id),
                ]),
                api_tags,
            ),
            "annotations": {
                "service.beta.kubernetes.io/aws-load-balancer-type": "nlb"
            },
        },
        "spec": {
            "type": "LoadBalancer",
            "externalTrafficPolicy": "Cluster",
            "selector": {
                "app": "hawkeye",
                "watcher_id": watcher_id,
            },
            "ports": [
                {
                    "name": "video-feed",
                    "protocol": "UDP",
                    "port": ingest_port,
                    "targetPort": ingest_port
                }
            ]
        },
    }))
    .unwrap()
}

#[cfg(test)]
mod tests {
    use crate::templates::{
        build_configmap, build_deployment, build_service, configmap_name, deployment_name,
        service_name, sys_api_tags_concat, K8S_LABEL_API_TAG_PREFIX,
    };
    use k8s_openapi::api::apps::v1::{Deployment, DeploymentSpec};
    use k8s_openapi::api::core::v1::{
        ConfigMap, ConfigMapKeySelector, ConfigMapVolumeSource, Container, ContainerPort, EnvVar,
        EnvVarSource, KeyToPath, PodSpec, PodTemplateSpec, ResourceRequirements, Service,
        ServicePort, ServiceSpec, Volume, VolumeMount,
    };
    use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelector, ObjectMeta};
    use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int;
    use std::collections::{BTreeMap, HashMap};

    #[test]
    fn build_configmap_happy_path() {
        let watcher_id = "abc123";
        let contents = "<some content>";
        let api_tags = HashMap::from([("foo".to_string(), "bar".to_string())]);
        let config_map = build_configmap(watcher_id, contents, Some(api_tags).as_ref());
        assert_eq!(
            config_map,
            ConfigMap {
                binary_data: None,
                data: Some(BTreeMap::from([
                    ("log_level".to_string(), "INFO".to_string()),
                    ("watcher.json".to_string(), "<some content>".to_string()),
                ])),
                immutable: None,
                metadata: ObjectMeta {
                    annotations: None,
                    cluster_name: None,
                    creation_timestamp: None,
                    deletion_grace_period_seconds: None,
                    deletion_timestamp: None,
                    finalizers: None,
                    generate_name: None,
                    generation: None,
                    labels: Some(BTreeMap::from([
                        ("app".to_string(), "hawkeye".to_string()),
                        (
                            format!("{}foo", K8S_LABEL_API_TAG_PREFIX).to_string(),
                            "bar".to_string()
                        ),
                        ("watcher_id".to_string(), "abc123".to_string()),
                    ])),
                    managed_fields: None,
                    name: Some("hawkeye-config-abc123".to_string()),
                    namespace: None,
                    owner_references: None,
                    resource_version: None,
                    self_link: None,
                    uid: None,
                }
            }
        )
    }

    #[test]
    fn build_deployment_happy_path() {
        let watcher_id = "abc123";
        let ingest_port = 4200;
        let api_tags = HashMap::from([
            ("foo".to_string(), "bar".to_string()),
            ("chicken".to_string(), "wing".to_string()),
        ]);
        let deployment = build_deployment(watcher_id, ingest_port, Some(&api_tags));
        assert_eq!(
            deployment,
            Deployment {
                metadata: ObjectMeta {
                    annotations: None,
                    cluster_name: None,
                    creation_timestamp: None,
                    deletion_grace_period_seconds: None,
                    deletion_timestamp: None,
                    finalizers: None,
                    generate_name: None,
                    generation: None,
                    labels: Some(BTreeMap::from([
                        ("app".to_string(), "hawkeye".to_string()),
                        (
                            format!("{}chicken", K8S_LABEL_API_TAG_PREFIX).to_string(),
                            "wing".to_string()
                        ),
                        (
                            format!("{}foo", K8S_LABEL_API_TAG_PREFIX).to_string(),
                            "bar".to_string()
                        ),
                        ("target_status".to_string(), "Ready".to_string()),
                        ("watcher_id".to_string(), "abc123".to_string()),
                    ])),
                    managed_fields: None,
                    name: Some("hawkeye-deploy-abc123".to_string()),
                    namespace: None,
                    owner_references: None,
                    resource_version: None,
                    self_link: None,
                    uid: None,
                },
                spec: Some(DeploymentSpec {
                    min_ready_seconds: None,
                    paused: None,
                    progress_deadline_seconds: None,
                    replicas: Some(0),
                    revision_history_limit: None,
                    selector: LabelSelector {
                        match_expressions: None,
                        match_labels: Some(BTreeMap::from([
                            ("app".to_string(), "hawkeye".to_string()),
                            ("watcher_id".to_string(), "abc123".to_string()),
                        ])),
                    },
                    strategy: None,
                    template: PodTemplateSpec {
                        metadata: Some(ObjectMeta {
                            annotations: Some(BTreeMap::from([
                                ("prometheus.io/path".to_string(), "metrics".to_string()),
                                ("prometheus.io/port".to_string(), "4200".to_string()),
                                ("prometheus.io/scrape".to_string(), "true".to_string()),
                            ])),
                            cluster_name: None,
                            creation_timestamp: None,
                            deletion_grace_period_seconds: None,
                            deletion_timestamp: None,
                            finalizers: None,
                            generate_name: None,
                            generation: None,
                            labels: Some(BTreeMap::from([
                                ("app".to_string(), "hawkeye".to_string()),
                                (
                                    format!("{}chicken", K8S_LABEL_API_TAG_PREFIX).to_string(),
                                    "wing".to_string()
                                ),
                                (
                                    format!("{}foo", K8S_LABEL_API_TAG_PREFIX).to_string(),
                                    "bar".to_string()
                                ),
                                ("prometheus.io/path".to_string(), "metrics".to_string()),
                                ("prometheus.io/port".to_string(), "4200".to_string()),
                                ("prometheus.io/scrape".to_string(), "true".to_string()),
                                ("target_status".to_string(), "Ready".to_string()),
                                ("watcher_id".to_string(), "abc123".to_string()),
                            ])),
                            managed_fields: None,
                            name: None,
                            namespace: None,
                            owner_references: None,
                            resource_version: None,
                            self_link: None,
                            uid: None,
                        }),
                        spec: Some(PodSpec {
                            active_deadline_seconds: None,
                            affinity: None,
                            automount_service_account_token: None,
                            containers: vec![Container {
                                args: Some(vec!["/config/watcher.json".to_string()]),
                                command: None,
                                env: Some(vec![EnvVar {
                                    name: "RUST_LOG".to_string(),
                                    value: None,
                                    value_from: Some(EnvVarSource {
                                        config_map_key_ref: Some(ConfigMapKeySelector {
                                            key: "log_level".to_string(),
                                            name: Some("hawkeye-config-abc123".to_string()),
                                            optional: None,
                                        }),
                                        field_ref: None,
                                        resource_field_ref: None,
                                        secret_key_ref: None,
                                    })
                                },]),
                                env_from: None,
                                image: Some("hawkeye-worker:latest".to_string()),
                                image_pull_policy: Some("IfNotPresent".to_string()),
                                lifecycle: None,
                                liveness_probe: None,
                                name: "hawkeye-app".to_string(),
                                ports: Some(vec![
                                    ContainerPort {
                                        container_port: 4200,
                                        host_ip: None,
                                        host_port: None,
                                        name: None,
                                        protocol: Some("UDP".to_string()),
                                    },
                                    ContainerPort {
                                        container_port: 4200,
                                        host_ip: None,
                                        host_port: None,
                                        name: None,
                                        protocol: Some("TCP".to_string()),
                                    },
                                ]),
                                readiness_probe: None,
                                resources: Some(ResourceRequirements {
                                    limits: Some(BTreeMap::from([
                                        ("cpu".to_string(), Quantity("2000m".to_string())),
                                        ("memory".to_string(), Quantity("100Mi".to_string())),
                                    ])),
                                    requests: Some(BTreeMap::from([
                                        ("cpu".to_string(), Quantity("1150m".to_string())),
                                        ("memory".to_string(), Quantity("50Mi".to_string())),
                                    ])),
                                }),
                                security_context: None,
                                startup_probe: None,
                                stdin: None,
                                stdin_once: None,
                                termination_message_path: None,
                                termination_message_policy: None,
                                tty: None,
                                volume_devices: None,
                                volume_mounts: Some(vec![VolumeMount {
                                    mount_path: "/config".to_string(),
                                    mount_propagation: None,
                                    name: "config".to_string(),
                                    read_only: Some(true),
                                    sub_path: None,
                                    sub_path_expr: None,
                                }]),
                                working_dir: None,
                            },],
                            dns_config: None,
                            dns_policy: Some("Default".to_string()),
                            enable_service_links: None,
                            ephemeral_containers: None,
                            host_aliases: None,
                            host_ipc: None,
                            host_network: None,
                            host_pid: None,
                            hostname: None,
                            image_pull_secrets: None,
                            init_containers: None,
                            node_name: None,
                            node_selector: None,
                            overhead: None,
                            preemption_policy: None,
                            priority: None,
                            priority_class_name: None,
                            readiness_gates: None,
                            restart_policy: Some("Always".to_string()),
                            runtime_class_name: None,
                            scheduler_name: None,
                            security_context: None,
                            service_account: None,
                            service_account_name: None,
                            set_hostname_as_fqdn: None,
                            share_process_namespace: None,
                            subdomain: None,
                            termination_grace_period_seconds: Some(5),
                            tolerations: None,
                            topology_spread_constraints: None,
                            volumes: Some(vec![Volume {
                                aws_elastic_block_store: None,
                                azure_disk: None,
                                azure_file: None,
                                cephfs: None,
                                cinder: None,
                                config_map: Some(ConfigMapVolumeSource {
                                    default_mode: None,
                                    items: Some(vec![KeyToPath {
                                        key: "watcher.json".to_string(),
                                        mode: None,
                                        path: "watcher.json".to_string(),
                                    },]),
                                    name: Some("hawkeye-config-abc123".to_string()),
                                    optional: None,
                                }),
                                csi: None,
                                downward_api: None,
                                empty_dir: None,
                                ephemeral: None,
                                fc: None,
                                flex_volume: None,
                                flocker: None,
                                gce_persistent_disk: None,
                                git_repo: None,
                                glusterfs: None,
                                host_path: None,
                                iscsi: None,
                                name: "config".to_string(),
                                nfs: None,
                                persistent_volume_claim: None,
                                photon_persistent_disk: None,
                                portworx_volume: None,
                                projected: None,
                                quobyte: None,
                                rbd: None,
                                scale_io: None,
                                secret: None,
                                storageos: None,
                                vsphere_volume: None,
                            },])
                        })
                    },
                }),
                status: None,
            }
        );
    }

    #[test]
    fn build_service_happy_path() {
        let watcher_id = "abc123";
        let ingest_port = 4200;
        let api_tags = HashMap::from([
            ("foo".to_string(), "bar".to_string()),
            ("chicken".to_string(), "wing".to_string()),
        ]);
        let service = build_service(watcher_id, ingest_port, Some(&api_tags));
        assert_eq!(
            service,
            Service {
                metadata: ObjectMeta {
                    annotations: Some(BTreeMap::from([(
                        "service.beta.kubernetes.io/aws-load-balancer-type".to_string(),
                        "nlb".to_string()
                    ),])),
                    cluster_name: None,
                    creation_timestamp: None,
                    deletion_grace_period_seconds: None,
                    deletion_timestamp: None,
                    finalizers: None,
                    generate_name: None,
                    generation: None,
                    labels: Some(BTreeMap::from([
                        ("app".to_string(), "hawkeye".to_string()),
                        (
                            format!("{}chicken", K8S_LABEL_API_TAG_PREFIX).to_string(),
                            "wing".to_string()
                        ),
                        (
                            format!("{}foo", K8S_LABEL_API_TAG_PREFIX).to_string(),
                            "bar".to_string()
                        ),
                        ("watcher_id".to_string(), "abc123".to_string()),
                    ])),
                    managed_fields: None,
                    name: Some("hawkeye-vid-svc-abc123".to_string()),
                    namespace: None,
                    owner_references: None,
                    resource_version: None,
                    self_link: None,
                    uid: None,
                },
                spec: Some(ServiceSpec {
                    allocate_load_balancer_node_ports: None,
                    cluster_ip: None,
                    cluster_ips: None,
                    external_ips: None,
                    external_name: None,
                    external_traffic_policy: Some("Cluster".to_string()),
                    health_check_node_port: None,
                    internal_traffic_policy: None,
                    ip_families: None,
                    ip_family_policy: None,
                    load_balancer_class: None,
                    load_balancer_ip: None,
                    load_balancer_source_ranges: None,
                    ports: Some(vec![ServicePort {
                        app_protocol: None,
                        name: Some("video-feed".to_string()),
                        node_port: None,
                        port: 4200,
                        protocol: Some("UDP".to_string()),
                        target_port: Some(Int(4200)),
                    },]),
                    publish_not_ready_addresses: None,
                    selector: Some(BTreeMap::from([
                        ("app".to_string(), "hawkeye".to_string()),
                        ("watcher_id".to_string(), "abc123".to_string()),
                    ])),
                    session_affinity: None,
                    session_affinity_config: None,
                    type_: Some("LoadBalancer".to_string())
                }),
                status: None,
            }
        )
    }

    #[test]
    fn configmap_name_generates_correct_name() {
        assert_eq!(
            configmap_name("jupiter"),
            "hawkeye-config-jupiter".to_string()
        );
    }

    #[test]
    fn deployment_name_generates_correct_name() {
        assert_eq!(
            deployment_name("saturn"),
            "hawkeye-deploy-saturn".to_string()
        );
    }

    #[test]
    fn service_name_generates_correct_name() {
        assert_eq!(service_name("mars"), "hawkeye-vid-svc-mars".to_string());
    }

    #[test]
    fn sys_api_tags_concats_valid_sys_tags_and_valid_custom_tags() {
        let system_tags = HashMap::from([("very", "important")]);
        let api_tags = Some(HashMap::from([
            ("never gonna give".to_string(), "you up".to_string()),
            ("never gonna let".to_string(), "you dowwnnn".to_string()),
        ]));
        let concat = sys_api_tags_concat(&system_tags, api_tags.as_ref());
        assert_eq!(
            concat,
            HashMap::from([
                ("very".to_string(), "important".to_string()),
                (
                    format!("{}never gonna give", K8S_LABEL_API_TAG_PREFIX).to_string(),
                    "you up".to_string()
                ),
                (
                    format!("{}never gonna let", K8S_LABEL_API_TAG_PREFIX).to_string(),
                    "you dowwnnn".to_string()
                ),
            ])
        );
    }

    #[test]
    fn sys_api_tags_concats_sys_empty_tags_and_valid_custom_tags() {
        let system_tags = HashMap::new();
        let api_tags = Some(HashMap::from([
            ("never gonna give".to_string(), "you up".to_string()),
            ("never gonna let".to_string(), "you dowwnnn".to_string()),
        ]));
        let concat = sys_api_tags_concat(&system_tags, api_tags.as_ref());
        assert_eq!(
            concat,
            HashMap::from([
                (
                    format!("{}never gonna give", K8S_LABEL_API_TAG_PREFIX).to_string(),
                    "you up".to_string()
                ),
                (
                    format!("{}never gonna let", K8S_LABEL_API_TAG_PREFIX).to_string(),
                    "you dowwnnn".to_string()
                ),
            ])
        );
    }

    #[test]
    fn sys_api_tags_concats_sys_valid_tags_and_empty_custom_tags() {
        let system_tags = HashMap::from([("very", "important")]);
        let api_tags = Some(HashMap::new());
        let concat = sys_api_tags_concat(&system_tags, api_tags.as_ref());
        assert_eq!(
            concat,
            HashMap::from([("very".to_string(), "important".to_string()),])
        );
    }

    #[test]
    fn sys_api_tags_concats_sys_empty_tags_and_empty_custom_tags() {
        let system_tags = HashMap::new();
        let api_tags = Some(HashMap::new());
        let concat = sys_api_tags_concat(&system_tags, api_tags.as_ref());
        assert_eq!(concat, HashMap::new());
    }
}
