use std::time::Duration;

use axum::async_trait;
use axum_sessions::async_session::{Result, Session, SessionStore};
use sqlx::SqlitePool;
use tokio::time;

#[derive(Debug, Clone)]
pub struct DbSessionStore {
    db: SqlitePool,
}

impl DbSessionStore {
    pub fn new(db: &SqlitePool) -> DbSessionStore {
        DbSessionStore { db: db.clone() }
    }

    pub async fn continuously_delete_expired(self) -> Result<()> {
        let mut interval = time::interval(Duration::from_secs(10));
        interval.tick().await; // skip immediate tick
        loop {
            interval.tick().await;
            self.delete_expired().await?;
        }
    }

    async fn delete_expired(&self) -> Result<()> {
        tracing::trace!("destroying expired sessions");
        sqlx::query!(
            r"
            delete from session
            where updated_at < date('now', '-1 day');
            ",
        )
        .execute(&self.db)
        .await?;
        Ok(())
    }
}

#[async_trait]
impl SessionStore for DbSessionStore {
    async fn load_session(&self, cookie_value: String) -> Result<Option<Session>> {
        let session_id = Session::id_from_cookie_value(&cookie_value)?;
        tracing::trace!(session_id, "loading session");
        Ok(sqlx::query!(
            r"
            select as_json
            from session
            where session_id = ?
            ",
            session_id,
        )
        .fetch_optional(&self.db)
        .await?
        .map(|r| serde_json::from_str::<Session>(&r.as_json))
        .transpose()?)
    }

    async fn store_session(&self, session: Session) -> Result<Option<String>> {
        let json = serde_json::to_string(&session)?;
        let session_id = session.id();
        tracing::trace!(session_id, "storing session");
        sqlx::query!(
            r"
            insert into session (session_id, as_json)
            values (?, ?)
            on conflict (session_id) do
            update set as_json = ?, updated_at = current_timestamp
            ",
            session_id,
            json,
            json,
        )
        .execute(&self.db)
        .await?;

        session.reset_data_changed();
        Ok(session.into_cookie_value())
    }

    /// Remove a session from the session store
    async fn destroy_session(&self, session: Session) -> Result {
        let session_id = session.id();
        tracing::trace!(session_id, "destroying session");
        sqlx::query!(
            r"
            delete from session
            where session_id = ?
            ",
            session_id,
        )
        .execute(&self.db)
        .await?;
        Ok(())
    }

    async fn clear_store(&self) -> Result {
        tracing::trace!("destroying all sessions");
        sqlx::query!(
            r"
            delete from session
            ",
        )
        .execute(&self.db)
        .await?;
        Ok(())
    }
}
