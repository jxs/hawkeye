use crate::{auth, filters, handlers};
use eyre::ErrReport;
use hawkeye_core::models::Watcher;
use kube::Client;
use serde::Serialize;
use std::fmt::Display;
use warp::hyper::StatusCode;
use warp::reply::Response;
use warp::{reject, Filter};

/// API root for v1
pub fn v1(
    client: Client,
) -> impl Filter<Extract = impl warp::Reply, Error = std::convert::Infallible> + Clone {
    watchers_list(client.clone())
        .or(watcher_create(client.clone()))
        .or(watcher_get(client.clone()))
        .or(watcher_update(client.clone()))
        .or(watcher_delete(client.clone()))
        .or(watcher_upgrade(client.clone()))
        .or(watcher_start(client.clone()))
        .or(watcher_stop(client.clone()))
        .or(watcher_video_frame(client.clone()))
        .or(migration_for_multislate(client.clone()))
        .or(healthcheck(client))
        .recover(handle_rejection)
}

/// GET /v1/watchers
pub fn watchers_list(
    client: Client,
) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path!("v1" / "watchers")
        .and(auth::verify())
        .and(warp::get())
        .and(with_client(client))
        .and_then(handlers::list_watchers)
}

/// POST /v1/watchers
pub fn watcher_create(
    client: Client,
) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path!("v1" / "watchers")
        .and(auth::verify())
        .and(warp::post())
        .and(json_body())
        .and(with_client(client))
        .and_then(handlers::create_watcher)
}

/// PATCH /v1/watchers/{id}
pub fn watcher_update(
    client: Client,
) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path!("v1" / "watchers" / String)
        .and(auth::verify())
        .and(warp::put())
        .and(json_body())
        .and(with_client(client))
        .and_then(handlers::update_watcher)
}

/// GET /v1/watchers/{id}
pub fn watcher_get(
    client: Client,
) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path!("v1" / "watchers" / String)
        .and(auth::verify())
        .and(warp::get())
        .and(with_client(client))
        .and_then(handlers::get_watcher)
}

/// DELETE /v1/watchers/{id}
pub fn watcher_delete(
    client: Client,
) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path!("v1" / "watchers" / String)
        .and(auth::verify())
        .and(warp::delete())
        .and(with_client(client))
        .and_then(handlers::delete_watcher)
}

/// POST /v1/watchers/{id}/upgrade
pub fn watcher_upgrade(
    client: Client,
) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path!("v1" / "watchers" / String / "upgrade")
        .and(auth::verify())
        .and(warp::post())
        .and(with_client(client))
        .and_then(handlers::upgrade_watcher)
}

/// POST /v1/watchers/{id}/start
pub fn watcher_start(
    client: Client,
) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path!("v1" / "watchers" / String / "start")
        .and(auth::verify())
        .and(warp::post())
        .and(with_client(client))
        .and_then(handlers::start_watcher)
}

/// POST /v1/watchers/{id}/stop
pub fn watcher_stop(
    client: Client,
) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path!("v1" / "watchers" / String / "stop")
        .and(auth::verify())
        .and(warp::post())
        .and(with_client(client))
        .and_then(handlers::stop_watcher)
}

/// GET /v1/watchers/{id}/video-frame
pub fn watcher_video_frame(
    client: Client,
) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path!("v1" / "watchers" / String / "video-frame")
        .and(warp::get())
        .and(with_client(client))
        .and_then(handlers::get_video_frame)
}

/// POST /v1/migration/1/multislate
pub fn migration_for_multislate(
    client: Client,
) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path!("v1" / "migrations" / "1" / "multislate")
        .and(auth::verify())
        .and(warp::post())
        .and(with_client(client))
        .and_then(handlers::migrate_to_multislate)
}

/// GET /healthcheck
pub fn healthcheck(
    client: Client,
) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path("healthcheck")
        .and(warp::get())
        .and(with_client(client))
        .and_then(handlers::healthcheck)
}

fn with_client(
    client: Client,
) -> impl Filter<Extract = (Client,), Error = std::convert::Infallible> + Clone {
    warp::any().map(move || client.clone())
}

fn json_body() -> impl Filter<Extract = (Watcher,), Error = warp::Rejection> + Clone {
    // When accepting a body, we want a JSON body
    // (and to reject huge payloads)...
    warp::body::content_length_limit(1024 * 16).and(warp::body::json())
}

/// An API error serializable to JSON.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ErrorResponse {
    pub message: String,
}

impl reject::Reject for ErrorResponse {}

impl Display for ErrorResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl ErrorResponse {
    pub fn new<S: AsRef<str>>(message: S) -> Self {
        Self {
            message: message.as_ref().to_string(),
        }
    }
}

impl From<eyre::ErrReport> for ErrorResponse {
    fn from(err: ErrReport) -> Self {
        let e = match err.downcast::<ErrorResponse>() {
            Ok(e) => return e,
            Err(e) => e,
        };

        ErrorResponse::new(e.to_string().as_str())
    }
}

impl warp::Reply for ErrorResponse {
    fn into_response(self) -> Response {
        let json = warp::reply::json(&self);
        warp::reply::with_status(json, StatusCode::UNPROCESSABLE_ENTITY).into_response()
    }
}

async fn handle_rejection(
    err: warp::Rejection,
) -> Result<impl warp::Reply, std::convert::Infallible> {
    log::debug!("Rejection = {:?}", err);
    let mut message = "".to_string();
    let code;

    if err.is_not_found() {
        code = StatusCode::NOT_FOUND;
    } else if let Some(err) = err.find::<warp::filters::body::BodyDeserializeError>() {
        code = StatusCode::BAD_REQUEST;
        message = err.to_string();
    } else if err.find::<auth::NoAuth>().is_some() {
        code = StatusCode::UNAUTHORIZED;
    } else if err.find::<filters::InternalError>().is_some() {
        code = StatusCode::INTERNAL_SERVER_ERROR;
    } else if let Some(e) = err.find::<ErrorResponse>() {
        code = StatusCode::UNPROCESSABLE_ENTITY;
        message = e.message.to_owned();
    } else if let Some(missing) = err.find::<warp::reject::MissingHeader>() {
        if missing.name() == "authorization" {
            code = StatusCode::UNAUTHORIZED;
        } else {
            code = StatusCode::BAD_REQUEST;
        }
    } else if err.find::<warp::reject::MethodNotAllowed>().is_some() {
        code = StatusCode::METHOD_NOT_ALLOWED;
    } else {
        log::debug!("Unhandled rejection: {:?}", err);
        code = StatusCode::INTERNAL_SERVER_ERROR;
    }

    // Use the status code's text value as the default message if none supplied.
    message.is_empty().then(|| {
        message = match &code.canonical_reason() {
            Some(reason) => reason.to_string(),
            None => "an unknown server error has occurred".to_string(),
        }
    });
    let json = warp::reply::json(&ErrorResponse { message });
    Ok(warp::reply::with_status(json, code))
}

#[derive(Debug)]
pub struct InternalError;
impl warp::reject::Reject for InternalError {}
