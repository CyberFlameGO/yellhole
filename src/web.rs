use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use askama::Template;
use axum::http::{self, StatusCode, Uri};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use futures::Future;
use sqlx::SqlitePool;
use tokio::io;
use tower_http::add_extension::AddExtensionLayer;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::sensitive_headers::SetSensitiveRequestHeadersLayer;
use tower_http::trace::TraceLayer;

mod admin;
mod asset;
mod feed;

#[derive(Debug, Clone)]
pub struct Context {
    db: SqlitePool,
    name: String,
    author: String,
    images_dir: PathBuf,
    uploads_dir: PathBuf,
}

impl Context {
    pub async fn new(
        db: SqlitePool,
        name: String,
        author: String,
        data_dir: impl AsRef<Path>,
    ) -> Result<Context, io::Error> {
        // Create the images and uploads directories, if necessary.
        let mut images_dir = data_dir.as_ref().to_path_buf();
        images_dir.push("images");
        tracing::info!(?images_dir, "creating directory");
        tokio::fs::create_dir_all(&images_dir).await?;

        let mut uploads_dir = data_dir.as_ref().to_path_buf();
        uploads_dir.push("uploads");
        tracing::info!(?uploads_dir, "creating directory");
        tokio::fs::create_dir_all(&uploads_dir).await?;

        Ok(Context { db, name, author, images_dir, uploads_dir })
    }

    pub async fn serve(
        self,
        addr: &SocketAddr,
        shutdown_hook: impl Future<Output = ()>,
    ) -> anyhow::Result<()> {
        tracing::info!(%addr, url=format!("http://{addr}"), "starting server");

        let app = feed::router()
            .merge(admin::router())
            .merge(asset::router(&self.images_dir))
            .layer(AddExtensionLayer::new(self))
            .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
            .layer(SetSensitiveRequestHeadersLayer::new(std::iter::once(http::header::COOKIE)))
            .layer(TraceLayer::new_for_http())
            .layer(SetSensitiveRequestHeadersLayer::new(std::iter::once(http::header::SET_COOKIE)))
            .layer(PropagateRequestIdLayer::x_request_id())
            .layer(middleware::from_fn(handle_errors));

        axum::Server::bind(addr)
            .serve(app.into_make_service())
            .with_graceful_shutdown(shutdown_hook)
            .await?;

        Ok(())
    }
}

#[derive(Debug, Template)]
#[template(path = "error.html")]
struct ErrorPage {
    uri: Uri,
    status: StatusCode,
}

async fn handle_errors<B>(req: http::Request<B>, next: Next<B>) -> Result<Response, StatusCode> {
    let uri = req.uri().clone();
    let resp = next.run(req).await;
    if resp.status().is_client_error() || resp.status().is_server_error() {
        return Ok(Page(ErrorPage { uri, status: resp.status() }).into_response());
    }
    Ok(resp)
}

#[derive(Debug)]
pub struct Page<T: Template>(T);

impl<T: Template> IntoResponse for Page<T> {
    fn into_response(self) -> Response {
        match self.0.render() {
            Ok(body) => Html(body).into_response(),
            Err(err) => {
                tracing::error!(?err, "unable to render template");
                http::StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        }
    }
}
