use actix_web::web::Data;
use async_sqlite::{
    Pool, rusqlite,
    rusqlite::{Error, Row},
};
use atrium_api::types::string::Did;
use chrono::{DateTime, Datelike, Utc};
use rusqlite::types::Type;
use serde::{Deserialize, Serialize};
use std::{fmt::Debug, sync::Arc};

/// Creates the tables in the db.
pub async fn create_tables_in_database(pool: &Pool) -> Result<(), async_sqlite::Error> {
    pool.conn(move |conn| {
        conn.execute("PRAGMA foreign_keys = ON", []).unwrap();

        // status
        conn.execute(
            "CREATE TABLE IF NOT EXISTS status (
            uri TEXT PRIMARY KEY,
            authorDid TEXT NOT NULL,
            status TEXT NOT NULL,
            createdAt INTEGER  NOT NULL,
            indexedAt INTEGER  NOT NULL
        )",
            [],
        )
        .unwrap();

        // auth_session
        conn.execute(
            "CREATE TABLE IF NOT EXISTS auth_session (
            key TEXT PRIMARY KEY,
            session TEXT NOT NULL
        )",
            [],
        )
        .unwrap();

        // auth_state
        conn.execute(
            "CREATE TABLE IF NOT EXISTS auth_state (
            key TEXT PRIMARY KEY,
            state TEXT NOT NULL
        )",
            [],
        )
        .unwrap();
        Ok(())
    })
    .await?;
    Ok(())
}

///Status table datatype
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StatusFromDb {
    pub uri: String,
    pub author_did: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub indexed_at: DateTime<Utc>,
    pub handle: Option<String>,
}

//Status methods
impl StatusFromDb {
    /// Creates a new [StatusFromDb]
    pub fn new(uri: String, author_did: String, status: String) -> Self {
        let now = chrono::Utc::now();
        Self {
            uri,
            author_did,
            status,
            created_at: now,
            indexed_at: now,
            handle: None,
        }
    }

    /// Helper to map from [Row] to [StatusDb]
    fn map_from_row(row: &Row) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            uri: row.get(0)?,
            author_did: row.get(1)?,
            status: row.get(2)?,
            //DateTimes are stored as INTEGERS then parsed into a DateTime<UTC>
            created_at: {
                let timestamp: i64 = row.get(3)?;
                DateTime::from_timestamp(timestamp, 0).ok_or_else(|| {
                    Error::InvalidColumnType(3, "Invalid timestamp".parse().unwrap(), Type::Text)
                })?
            },
            //DateTimes are stored as INTEGERS then parsed into a DateTime<UTC>
            indexed_at: {
                let timestamp: i64 = row.get(4)?;
                DateTime::from_timestamp(timestamp, 0).ok_or_else(|| {
                    Error::InvalidColumnType(4, "Invalid timestamp".parse().unwrap(), Type::Text)
                })?
            },
            handle: None,
        })
    }

    /// Helper for the UI to see if indexed_at date is today or not
    pub fn is_today(&self) -> bool {
        let now = Utc::now();

        self.indexed_at.day() == now.day()
            && self.indexed_at.month() == now.month()
            && self.indexed_at.year() == now.year()
    }

    /// Saves the [StatusDb]
    pub async fn save(&self, pool: Data<Arc<Pool>>) -> Result<(), async_sqlite::Error> {
        let cloned_self = self.clone();
        pool.conn(move |conn| {
            Ok(conn.execute(
                "INSERT INTO status (uri, authorDid, status, createdAt, indexedAt) VALUES (?1, ?2, ?3, ?4, ?5)",
                [
                    &cloned_self.uri,
                    &cloned_self.author_did,
                    &cloned_self.status,
                    &cloned_self.created_at.timestamp().to_string(),
                    &cloned_self.indexed_at.timestamp().to_string(),
                ],
            )?)
        })
            .await?;
        Ok(())
    }

    /// Saves or updates a status by its did(uri)
    pub async fn save_or_update(&self, pool: &Pool) -> Result<(), async_sqlite::Error> {
        let cloned_self = self.clone();
        pool.conn(move |conn| {
            //We check to see if the session already exists, if so we need to update not insert
            let mut stmt = conn.prepare("SELECT COUNT(*) FROM status WHERE uri = ?1")?;
            let count: i64 = stmt.query_row([&cloned_self.uri], |row| row.get(0))?;
            match count > 0 {
                true => {
                    let mut update_stmt =
                        conn.prepare("UPDATE status SET status = ?2, indexedAt = ? WHERE uri = ?1")?;
                    update_stmt.execute([&cloned_self.uri, &cloned_self.status, &cloned_self.indexed_at.timestamp().to_string()])?;
                    Ok(())
                }
                false => {
                    conn.execute(
                        "INSERT INTO status (uri, authorDid, status, createdAt, indexedAt) VALUES (?1, ?2, ?3, ?4, ?5)",
                        [
                            &cloned_self.uri,
                            &cloned_self.author_did,
                            &cloned_self.status,
                            &cloned_self.created_at.timestamp().to_string(),
                            &cloned_self.indexed_at.timestamp().to_string(),
                        ],
                    )?;
                    Ok(())
                }
            }
        })
        .await?;
        Ok(())
    }
    pub async fn delete_by_uri(pool: &Pool, uri: String) -> Result<(), async_sqlite::Error> {
        pool.conn(move |conn| {
            let mut stmt = conn.prepare("DELETE FROM status WHERE uri = ?1")?;
            stmt.execute([&uri])
        })
        .await?;
        Ok(())
    }

    /// Loads the last 10 statuses we have saved
    pub async fn load_latest_statuses(
        pool: &Data<Arc<Pool>>,
    ) -> Result<Vec<Self>, async_sqlite::Error> {
        Ok(pool
            .conn(move |conn| {
                let mut stmt =
                    conn.prepare("SELECT * FROM status ORDER BY indexedAt DESC LIMIT 10")?;
                let status_iter = stmt
                    .query_map([], |row| Ok(Self::map_from_row(row).unwrap()))
                    .unwrap();

                let mut statuses = Vec::new();
                for status in status_iter {
                    statuses.push(status?);
                }
                Ok(statuses)
            })
            .await?)
    }

    /// Loads the logged-in users current status
    pub async fn my_status(
        pool: &Data<Arc<Pool>>,
        did: &str,
    ) -> Result<Option<Self>, async_sqlite::Error> {
        let did = did.to_string();
        pool.conn(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT * FROM status WHERE authorDid = ?1 ORDER BY createdAt DESC LIMIT 1",
            )?;
            stmt.query_row([did.as_str()], |row| Self::map_from_row(row))
                .map(Some)
                .or_else(|err| {
                    if err == rusqlite::Error::QueryReturnedNoRows {
                        Ok(None)
                    } else {
                        Err(err)
                    }
                })
        })
        .await
    }

    /// ui helper to show a handle or did if the handle cannot be found
    pub fn author_display_name(&self) -> String {
        match self.handle.as_ref() {
            Some(handle) => handle.to_string(),
            None => self.author_did.to_string(),
        }
    }
}

/// AuthSession table data type
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AuthSession {
    pub key: String,
    pub session: String,
}

impl AuthSession {
    /// Creates a new [AuthSession]
    pub fn new<V>(key: String, session: V) -> Self
    where
        V: Serialize,
    {
        let session = serde_json::to_string(&session).unwrap();
        Self {
            key: key.to_string(),
            session,
        }
    }

    /// Helper to map from [Row] to [AuthSession]
    fn map_from_row(row: &Row) -> Result<Self, Error> {
        let key: String = row.get(0)?;
        let session: String = row.get(1)?;
        Ok(Self { key, session })
    }

    /// Gets a session by the users did(key)
    pub async fn get_by_did(pool: &Pool, did: String) -> Result<Option<Self>, async_sqlite::Error> {
        let did = Did::new(did).unwrap();
        pool.conn(move |conn| {
            let mut stmt = conn.prepare("SELECT * FROM auth_session WHERE key = ?1")?;
            stmt.query_row([did.as_str()], |row| Self::map_from_row(row))
                .map(Some)
                .or_else(|err| {
                    if err == Error::QueryReturnedNoRows {
                        Ok(None)
                    } else {
                        Err(err)
                    }
                })
        })
        .await
    }

    /// Saves or updates the session by its did(key)
    pub async fn save_or_update(&self, pool: &Pool) -> Result<(), async_sqlite::Error> {
        let cloned_self = self.clone();
        pool.conn(move |conn| {
            //We check to see if the session already exists, if so we need to update not insert
            let mut stmt = conn.prepare("SELECT COUNT(*) FROM auth_session WHERE key = ?1")?;
            let count: i64 = stmt.query_row([&cloned_self.key], |row| row.get(0))?;
            match count > 0 {
                true => {
                    let mut update_stmt =
                        conn.prepare("UPDATE auth_session SET session = ?2 WHERE key = ?1")?;
                    update_stmt.execute([&cloned_self.key, &cloned_self.session])?;
                    Ok(())
                }
                false => {
                    conn.execute(
                        "INSERT INTO auth_session (key, session) VALUES (?1, ?2)",
                        [&cloned_self.key, &cloned_self.session],
                    )?;
                    Ok(())
                }
            }
        })
        .await?;
        Ok(())
    }

    /// Deletes the session by did
    pub async fn delete_by_did(pool: &Pool, did: String) -> Result<(), async_sqlite::Error> {
        pool.conn(move |conn| {
            let mut stmt = conn.prepare("DELETE FROM auth_session WHERE key = ?1")?;
            stmt.execute([&did])
        })
        .await?;
        Ok(())
    }

    /// Deletes all the sessions
    pub async fn delete_all(pool: &Pool) -> Result<(), async_sqlite::Error> {
        pool.conn(move |conn| {
            let mut stmt = conn.prepare("DELETE FROM auth_session")?;
            stmt.execute([])
        })
        .await?;
        Ok(())
    }
}

/// AuthState table datatype
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AuthState {
    pub key: String,
    pub state: String,
}

impl AuthState {
    /// Creates a new [AuthState]
    pub fn new<V>(key: String, state: V) -> Self
    where
        V: Serialize,
    {
        let state = serde_json::to_string(&state).unwrap();
        Self {
            key: key.to_string(),
            state,
        }
    }

    /// Helper to map from [Row] to [AuthState]
    fn map_from_row(row: &Row) -> Result<Self, Error> {
        let key: String = row.get(0)?;
        let state: String = row.get(1)?;
        Ok(Self { key, state })
    }

    /// Gets a state by the users key
    pub async fn get_by_key(pool: &Pool, key: String) -> Result<Option<Self>, async_sqlite::Error> {
        pool.conn(move |conn| {
            let mut stmt = conn.prepare("SELECT * FROM auth_state WHERE key = ?1")?;
            stmt.query_row([key.as_str()], |row| Self::map_from_row(row))
                .map(Some)
                .or_else(|err| {
                    if err == Error::QueryReturnedNoRows {
                        Ok(None)
                    } else {
                        Err(err)
                    }
                })
        })
        .await
    }

    /// Saves or updates the state by its key
    pub async fn save_or_update(&self, pool: &Pool) -> Result<(), async_sqlite::Error> {
        let cloned_self = self.clone();
        pool.conn(move |conn| {
            //We check to see if the state already exists, if so we need to update
            let mut stmt = conn.prepare("SELECT COUNT(*) FROM auth_state WHERE key = ?1")?;
            let count: i64 = stmt.query_row([&cloned_self.key], |row| row.get(0))?;
            match count > 0 {
                true => {
                    let mut update_stmt =
                        conn.prepare("UPDATE auth_state SET state = ?2 WHERE key = ?1")?;
                    update_stmt.execute([&cloned_self.key, &cloned_self.state])?;
                    Ok(())
                }
                false => {
                    conn.execute(
                        "INSERT INTO auth_state (key, state) VALUES (?1, ?2)",
                        [&cloned_self.key, &cloned_self.state],
                    )?;
                    Ok(())
                }
            }
        })
        .await?;
        Ok(())
    }

    pub async fn delete_by_key(pool: &Pool, key: String) -> Result<(), async_sqlite::Error> {
        pool.conn(move |conn| {
            let mut stmt = conn.prepare("DELETE FROM auth_state WHERE key = ?1")?;
            stmt.execute([&key])
        })
        .await?;
        Ok(())
    }

    pub async fn delete_all(pool: &Pool) -> Result<(), async_sqlite::Error> {
        pool.conn(move |conn| {
            let mut stmt = conn.prepare("DELETE FROM auth_state")?;
            stmt.execute([])
        })
        .await?;
        Ok(())
    }
}
