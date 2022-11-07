use std::time::Duration;

use askama::Template;
use axum::extract::{Path, Query};
use axum::routing::get;
use axum::{Extension, Router};
use serde::Deserialize;

use super::{CacheControl, Context, Html, WebError};
use crate::models::Note;

pub fn router() -> Router {
    Router::new()
        .route("/", get(index))
        .route("/notes/:year/:month", get(month))
        .route("/note/:note_id", get(single))
}

#[derive(Debug, Template)]
#[template(path = "feed.html")]
struct FeedPage {
    notes: Vec<Note>,
}

#[derive(Debug, Deserialize)]
struct IndexOpts {
    n: Option<u16>,
}

async fn index(
    ctx: Extension<Context>,
    opts: Query<IndexOpts>,
) -> Result<Html<FeedPage>, WebError> {
    let notes = Note::most_recent(&ctx.db, opts.n.unwrap_or(100)).await?;

    Ok(Html(FeedPage { notes }, DEFAULT_CACHING))
}

async fn month(
    ctx: Extension<Context>,
    Path((year, month)): Path<(i32, u32)>,
) -> Result<Html<FeedPage>, WebError> {
    let notes = Note::month(&ctx.db, year, month).await?.ok_or(WebError::NotFound)?;
    Ok(Html(FeedPage { notes }, DEFAULT_CACHING))
}

async fn single(
    ctx: Extension<Context>,
    Path(note_id): Path<String>,
) -> Result<Html<FeedPage>, WebError> {
    let note = Note::by_id(&ctx.db, &note_id).await?.ok_or(WebError::NotFound)?;
    Ok(Html(FeedPage { notes: vec![note] }, CacheControl::Immutable))
}

const DEFAULT_CACHING: CacheControl = CacheControl::MaxAge(Duration::from_secs(60 * 5));
