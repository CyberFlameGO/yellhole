use chrono::NaiveDateTime;
use sqlx::SqlitePool;

#[derive(Debug)]
pub struct Image {
    pub image_id: String,
    pub created_at: NaiveDateTime,
}

impl Image {
    pub async fn most_recent(db: &SqlitePool, n: u16) -> Result<Vec<Image>, sqlx::Error> {
        sqlx::query_as!(
            Image,
            r"
            select image_id, created_at
            from image
            order by created_at desc
            limit ?
            ",
            n
        )
        .fetch_all(db)
        .await
    }

    pub async fn create(
        db: &SqlitePool,
        note_id: &str,
        original_filename: &str,
        content_type: &mime::Mime,
    ) -> Result<(), sqlx::Error> {
        let content_type = content_type.to_string();
        sqlx::query!(
            r"
            insert into image (image_id, original_filename, content_type) values (?, ?, ?)
            ",
            note_id,
            original_filename,
            content_type
        )
        .execute(db)
        .await?;
        Ok(())
    }
}
