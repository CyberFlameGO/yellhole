use std::path::{Path, PathBuf};
use std::process::ExitStatus;

use chrono::NaiveDateTime;
use sqlx::SqlitePool;
use tokio::io;
use tokio::process::Command;

#[derive(Debug)]
pub struct Image {
    pub image_id: String,
    pub original_file_ext: String,
    pub created_at: NaiveDateTime,
}

impl Image {
    pub async fn process_image(
        input: PathBuf,
        output: PathBuf,
        geometry: &'static str,
    ) -> io::Result<ExitStatus> {
        let mut proc = Command::new("magick")
            .arg(input)
            .arg("-auto-orient")
            .arg("-strip")
            .arg("-thumbnail")
            .arg(geometry)
            .arg(output)
            .spawn()?;
        proc.wait().await
    }

    pub fn original_path(uploads_dir: &Path, image_id: &str, content_type: &mime::Mime) -> PathBuf {
        let mut path = uploads_dir.to_path_buf();
        path.push(format!("{image_id}.orig.{}", content_type.subtype()));
        path
    }

    pub fn main_path(images_dir: &Path, image_id: &str) -> PathBuf {
        let mut path = images_dir.to_path_buf();
        path.push(format!("{image_id}.main.webp"));
        path
    }

    pub fn thumbnail_path(images_dir: &Path, image_id: &str) -> PathBuf {
        let mut path = images_dir.to_path_buf();
        path.push(format!("{image_id}.thumb.webp"));
        path
    }

    pub async fn create(db: &SqlitePool, content_type: &mime::Mime) -> Result<String, sqlx::Error> {
        let content_type = content_type.to_string();
        sqlx::query!(
            r"
            insert into image (content_type) values (?) returning image_id
            ",
            content_type
        )
        .fetch_one(db)
        .await
        .map(|r| r.image_id)
    }

    pub async fn mark_processed(db: &SqlitePool, image_id: &str) -> Result<(), sqlx::Error> {
        (sqlx::query!(
            r"
            update image set processed = true where image_id = ? 
            ",
            image_id
        )
        .execute(db)
        .await?
        .rows_affected()
            == 1)
            .then_some(())
            .ok_or(sqlx::Error::RowNotFound)
    }
}
