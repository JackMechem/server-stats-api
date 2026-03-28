use axum::middleware;
use axum::response::Redirect;
use axum::{Router, routing::get, routing::post};

mod auth;
mod config;
mod models;
mod routes;

#[tokio::main]
async fn main() {
    let protected = Router::new()
        .route("/stats", get(routes::stats::get_stats))
        .route(
            "/services/{service}/restart",
            post(routes::services::restart_service),
        )
        .route(
            "/services/{service}/start",
            post(routes::services::start_service),
        )
        .route(
            "/services/{service}/stop",
            post(routes::services::stop_service),
        )
        .route(
            "/services/{service}/logs",
            get(routes::services::service_logs),
        )
        .route("/system/reboot", post(routes::system::system_reboot))
        .route_layer(middleware::from_fn(auth::require_auth));

    let app = Router::new()
        .route("/", get(|| async { Redirect::permanent("/stats") }))
        .route("/auth/login", post(auth::post_login))
        .merge(protected);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3001")
        .await
        .unwrap();
    axum::serve(listener, app).await.unwrap();
}
