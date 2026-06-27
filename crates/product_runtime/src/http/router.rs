use axum::http::{header::CONTENT_TYPE, HeaderName, HeaderValue, Method};
use axum::middleware::from_fn_with_state;
use axum::routing::{delete, get, post};
use axum::Router;
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::app_state::ProductRuntimeState;
use crate::http::middleware::{require_runtime_token, RUNTIME_TOKEN_HEADER};
use crate::http::routes;

pub fn build_router(state: ProductRuntimeState, allowed_origins: Vec<String>) -> Router {
    Router::new()
        .route("/api/v1/runtime/meta", get(routes::runtime::meta))
        .route("/api/v1/runtime/health", get(routes::runtime::health))
        .route(
            "/api/v1/runtime/capabilities",
            get(routes::runtime::capabilities),
        )
        .route("/api/v1/runtime/events", get(routes::runtime::events))
        .route("/api/v1/runs", get(routes::runs::list))
        .route(
            "/api/v1/workspaces",
            get(routes::workspaces::list).post(routes::workspaces::create),
        )
        .route(
            "/api/v1/workspaces/activate",
            post(routes::workspaces::activate),
        )
        .route(
            "/api/v1/workspaces/:workspace_uid/archive",
            post(routes::workspaces::archive),
        )
        .route(
            "/api/v1/workspaces/:workspace_uid/containers",
            get(routes::workspaces::containers),
        )
        .route(
            "/api/v1/containers",
            get(routes::containers::list).post(routes::containers::create),
        )
        .route(
            "/api/v1/containers/archived",
            get(routes::containers::list_archived),
        )
        .route(
            "/api/v1/containers/:container_id",
            get(routes::containers::get_one).patch(routes::containers::update),
        )
        .route(
            "/api/v1/containers/:container_id/archive",
            post(routes::containers::archive),
        )
        .route(
            "/api/v1/containers/:container_id/activate",
            post(routes::containers::activate),
        )
        .route(
            "/api/v1/containers/:container_id/restore",
            post(routes::containers::restore),
        )
        .route(
            "/api/v1/containers/:container_id",
            delete(routes::containers::delete_one),
        )
        .route(
            "/api/v1/containers/:container_id/snapshot",
            get(routes::containers::snapshot),
        )
        .route(
            "/api/v1/containers/:container_id/messages",
            get(routes::containers::messages),
        )
        .route(
            "/api/v1/containers/:container_id/chat/threads",
            get(routes::chat::threads).post(routes::chat::create_thread),
        )
        .route(
            "/api/v1/chat/threads/:chat_thread_id/messages",
            get(routes::chat::messages),
        )
        .route(
            "/api/v1/chat/threads/:chat_thread_id/turns/stream",
            post(routes::chat::turn_stream),
        )
        .route(
            "/api/v1/chat/threads/:chat_thread_id/force-close",
            post(routes::chat::force_close),
        )
        .route(
            "/api/v1/containers/:container_id/tasks",
            get(routes::tasks::list),
        )
        .route(
            "/api/v1/containers/:container_id/tasks/stream",
            post(routes::tasks::start_stream),
        )
        .route("/api/v1/tasks/:task_id", get(routes::tasks::get_one))
        .route(
            "/api/v1/tasks/:task_id/messages",
            get(routes::tasks::messages),
        )
        .route(
            "/api/v1/tasks/:task_id/events/stream",
            get(routes::tasks::events_stream),
        )
        .route(
            "/api/v1/tasks/:task_id/input",
            post(routes::tasks::user_input),
        )
        .route(
            "/api/v1/tasks/:task_id/force-close",
            post(routes::tasks::force_close),
        )
        .route(
            "/api/v1/containers/:container_id/context-pack",
            get(routes::context_pack::get_current).post(routes::context_pack::save),
        )
        .route(
            "/api/v1/containers/:container_id/context-pack/estimate",
            post(routes::context_pack::estimate),
        )
        .route(
            "/api/v1/containers/:container_id/source-candidates",
            get(routes::context_pack::source_candidates),
        )
        .route(
            "/api/v1/containers/:container_id/artifact-targets",
            get(routes::artifacts::target_options),
        )
        .route(
            "/api/v1/model-config",
            get(routes::model_config::get).patch(routes::model_config::update),
        )
        .route(
            "/api/v1/settings",
            get(routes::settings::get).patch(routes::settings::update),
        )
        .route(
            "/api/v1/settings/provider",
            get(routes::settings::provider).patch(routes::settings::update_provider),
        )
        .route(
            "/api/v1/settings/provider/test",
            post(routes::settings::test_provider),
        )
        .route("/api/v1/diagnostics", get(routes::diagnostics::get))
        .layer(from_fn_with_state(state.clone(), require_runtime_token))
        .layer(runtime_cors_layer(allowed_origins))
        .with_state(state)
}

fn runtime_cors_layer(configured_origins: Vec<String>) -> CorsLayer {
    CorsLayer::new()
        .allow_origin(AllowOrigin::list(runtime_allowed_origins(
            configured_origins,
        )))
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([CONTENT_TYPE, HeaderName::from_static(RUNTIME_TOKEN_HEADER)])
}

fn runtime_allowed_origins(configured_origins: Vec<String>) -> Vec<HeaderValue> {
    let mut origins = vec![
        "http://tauri.localhost".to_string(),
        "https://tauri.localhost".to_string(),
        "tauri://localhost".to_string(),
    ];
    origins.extend(configured_origins);
    if let Ok(value) = std::env::var("SUPERNOVA_ALLOWED_ORIGINS") {
        origins.extend(
            value
                .split(',')
                .map(str::trim)
                .filter(|origin| !origin.is_empty())
                .map(str::to_string),
        );
    }
    origins
        .into_iter()
        .filter_map(|origin| HeaderValue::from_str(&origin).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::header::{ACCESS_CONTROL_ALLOW_ORIGIN, ACCESS_CONTROL_REQUEST_METHOD, ORIGIN};
    use axum::http::{Request, StatusCode};
    use std::path::PathBuf;
    use std::sync::Arc;
    use tower::ServiceExt;

    use crate::app_paths::{workspace_uid, AppPaths};
    use crate::services::Services;
    use crate::state::workspace_registry::now_ms;

    const TOKEN: &str = "test-runtime-token";

    #[tokio::test]
    async fn business_routes_reject_missing_and_invalid_runtime_token() {
        let app = build_router(test_state("reject"), Vec::new());

        let missing = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/settings")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);

        let invalid = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/settings")
                    .header(RUNTIME_TOKEN_HEADER, "wrong-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(invalid.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn sensitive_routes_reject_missing_runtime_token() {
        let app = build_router(test_state("reject_sensitive"), Vec::new());
        let cases = [
            (Method::GET, "/api/v1/workspaces", ""),
            (Method::PATCH, "/api/v1/settings/provider", "{}"),
            (
                Method::POST,
                "/api/v1/chat/threads/chat_1/turns/stream",
                "{}",
            ),
            (
                Method::POST,
                "/api/v1/containers/container_1/tasks/stream",
                "{}",
            ),
        ];

        for (method, uri, body) in cases {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(uri)
                        .header(CONTENT_TYPE, "application/json")
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{uri}");
        }
    }

    #[tokio::test]
    async fn business_routes_accept_valid_runtime_token_without_origin() {
        let app = build_router(test_state("accept"), Vec::new());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/settings")
                    .header(RUNTIME_TOKEN_HEADER, TOKEN)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn cors_rejects_untrusted_browser_origins() {
        let app = build_router(
            test_state("cors"),
            vec!["http://trusted.localhost:5173".into()],
        );

        let rejected = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::OPTIONS)
                    .uri("/api/v1/settings")
                    .header(ORIGIN, "http://evil.example")
                    .header(ACCESS_CONTROL_REQUEST_METHOD, "GET")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(rejected
            .headers()
            .get(ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none());

        let allowed = app
            .oneshot(
                Request::builder()
                    .method(Method::OPTIONS)
                    .uri("/api/v1/settings")
                    .header(ORIGIN, "http://trusted.localhost:5173")
                    .header(ACCESS_CONTROL_REQUEST_METHOD, "GET")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            allowed.headers().get(ACCESS_CONTROL_ALLOW_ORIGIN).unwrap(),
            "http://trusted.localhost:5173"
        );
    }

    #[tokio::test]
    async fn provider_settings_reject_deepseek_base_url_over_http() {
        let app = build_router(test_state("provider_base_url"), Vec::new());
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::PATCH)
                    .uri("/api/v1/settings/provider")
                    .header(RUNTIME_TOKEN_HEADER, TOKEN)
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"provider":"deepseek","api_base_url":"https://attacker.example","api_key":null}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn workspace_activation_rejects_root_fallback_over_http() {
        let app = build_router(test_state("workspace_root_fallback"), Vec::new());
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/workspaces/activate")
                    .header(RUNTIME_TOKEN_HEADER, TOKEN)
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"workspace_uid":null,"workspace_root":"C:/Users"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    fn test_state(name: &str) -> ProductRuntimeState {
        let config_root = temp_root(&format!("{name}_config"));
        let state_root = temp_root(&format!("{name}_state"));
        let workspace_root = temp_root(&format!("{name}_workspace"));
        let app_paths = AppPaths {
            app_config_root: config_root,
            app_state_root: state_root,
        };
        let workspace_uid = workspace_uid(&workspace_root);
        let workspace_state_root = app_paths.workspace_state_root(&workspace_uid);
        std::fs::create_dir_all(&workspace_state_root).unwrap();
        let services = Arc::new(
            Services::open(
                app_paths.clone(),
                workspace_root.clone(),
                workspace_uid.clone(),
                workspace_state_root,
                false,
            )
            .unwrap(),
        );
        ProductRuntimeState::new(
            app_paths,
            workspace_root,
            workspace_uid,
            services,
            TOKEN.into(),
        )
    }

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("supernova_router_{name}_{}", now_ms()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }
}
