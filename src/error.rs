use axum::{http::StatusCode, response::{IntoResponse, Response}, Json};
use serde_json::json;

pub struct AppError(pub anyhow::Error);

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        tracing::error!("request error: {:?}", self.0);
        let msg = self.0.to_string();
        (StatusCode::BAD_REQUEST, Json(json!({ "error": msg }))).into_response()
    }
}

impl<E> From<E> for AppError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        AppError(err.into())
    }
}

pub type AppResult<T> = Result<T, AppError>;
