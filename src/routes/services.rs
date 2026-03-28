use axum::{
    extract::Path, http::HeaderMap, http::StatusCode, response::IntoResponse,
    response::Json,
};
use tokio::process::Command;
use zbus::Connection;

use crate::auth;
use crate::config;
use crate::models::ActionResponse;

async fn systemd_action(action: &str, service: &str) -> (StatusCode, Json<ActionResponse>) {
    let unit = if service.contains('.') {
        service.to_string() // already has extension e.g. cloudflare-dyndns.timer
    } else {
        format!("{}.service", service)
    };

    let conn = match Connection::system().await {
        Ok(c) => c,
        Err(e) => return ActionResponse::err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };

    let method = match action {
        "restart" => "RestartUnit",
        "start" => "StartUnit",
        "stop" => "StopUnit",
        _ => return ActionResponse::err(StatusCode::BAD_REQUEST, "Invalid action"),
    };

    let result = conn
        .call_method(
            Some("org.freedesktop.systemd1"),
            "/org/freedesktop/systemd1",
            Some("org.freedesktop.systemd1.Manager"),
            method,
            &(unit.as_str(), "replace"),
        )
        .await;

    match result {
        Ok(_) => ActionResponse::ok(format!("{} {}ed successfully", service, action)),
        Err(e) => ActionResponse::err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

// POST /services/:service/restart
pub async fn restart_service(headers: HeaderMap, Path(service): Path<String>) -> impl IntoResponse {
    if !auth::verify_token(&headers) {
        return ActionResponse::err(StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }
    if !config::ALLOWED_SERVICES.contains(&service.as_str()) {
        return ActionResponse::err(StatusCode::BAD_REQUEST, "Service not allowed").into_response();
    }
    systemd_action("restart", &service).await.into_response()
}

// POST /services/:service/start
pub async fn start_service(headers: HeaderMap, Path(service): Path<String>) -> impl IntoResponse {
    if !auth::verify_token(&headers) {
        return ActionResponse::err(StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }
    if !config::ALLOWED_SERVICES.contains(&service.as_str()) {
        return ActionResponse::err(StatusCode::BAD_REQUEST, "Service not allowed").into_response();
    }
    systemd_action("start", &service).await.into_response()
}

// POST /services/:service/stop
pub async fn stop_service(headers: HeaderMap, Path(service): Path<String>) -> impl IntoResponse {
    if !auth::verify_token(&headers) {
        return ActionResponse::err(StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }
    if !config::ALLOWED_SERVICES.contains(&service.as_str()) {
        return ActionResponse::err(StatusCode::BAD_REQUEST, "Service not allowed").into_response();
    }
    systemd_action("stop", &service).await.into_response()
}

// GET /services/:service/logs
pub async fn service_logs(headers: HeaderMap, Path(service): Path<String>) -> impl IntoResponse {
    if !auth::verify_token(&headers) {
        return ActionResponse::err(StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }
    if !config::ALLOWED_SERVICES.contains(&service.as_str()) {
        return ActionResponse::err(StatusCode::BAD_REQUEST, "Service not allowed").into_response();
    }

    let out = Command::new("/run/current-system/sw/bin/journalctl")
        .args(["-u", &service, "-n", "100", "--no-pager"])
        .output()
        .await;

    match out {
        Ok(o) => (
            StatusCode::OK,
            Json(ActionResponse {
                success: true,
                message: String::new(),
                stdout: String::from_utf8_lossy(&o.stdout).to_string(),
                stderr: String::from_utf8_lossy(&o.stderr).to_string(),
            }),
        )
            .into_response(),
        Err(e) => {
            ActionResponse::err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()).into_response()
        }
    }
}
