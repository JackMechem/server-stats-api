use axum::{
    http::HeaderMap, http::StatusCode, response::IntoResponse,
};
use zbus::Connection;

use crate::auth;
use crate::models;

// POST /system/reboot
pub async fn system_reboot(headers: HeaderMap) -> impl IntoResponse {
    if !auth::verify_token(&headers) {
        return models::ActionResponse::err(StatusCode::UNAUTHORIZED, "Unauthorized")
            .into_response();
    }

    let conn = match Connection::system().await {
        Ok(c) => c,
        Err(e) => {
            return models::ActionResponse::err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string())
                .into_response();
        }
    };

    let result = conn
        .call_method(
            Some("org.freedesktop.login1"),
            "/org/freedesktop/login1",
            Some("org.freedesktop.login1.Manager"),
            "Reboot",
            &(false,), // false = don't ask for confirmation
        )
        .await;

    match result {
        Ok(_) => models::ActionResponse::ok("Rebooting...".to_string()).into_response(),
        Err(e) => models::ActionResponse::err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string())
            .into_response(),
    }
}
