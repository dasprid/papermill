use std::fs;
use std::path::Path;

use anyhow::Context;
use chrono::{DateTime, NaiveDate, Utc};
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode};

pub struct StateStore {
    pool: SqlitePool,
}

pub struct UploadRecord {
    pub source_id: String,
    pub external_id: String,
    pub paperless_task_id: String,
    pub invoice_issued_at: NaiveDate,
    pub uploaded_at: DateTime<Utc>,
}

impl StateStore {
    pub async fn open(path: &Path) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "Failed to create state database directory at {}",
                    parent.display()
                )
            })?;
        }

        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal);

        let pool = SqlitePool::connect_with(options)
            .await
            .with_context(|| format!("Failed to open state database at {}", path.display()))?;

        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .context("Failed to run state database migrations")?;

        Ok(Self { pool })
    }

    pub async fn is_uploaded(&self, source_id: &str, external_id: &str) -> anyhow::Result<bool> {
        let row = sqlx::query(
            "SELECT 1 FROM uploaded_invoices WHERE source_id = ?1 AND external_id = ?2",
        )
        .bind(source_id)
        .bind(external_id)
        .fetch_optional(&self.pool)
        .await
        .context("Failed to query uploaded_invoices")?;

        Ok(row.is_some())
    }

    pub async fn record_upload(&self, record: &UploadRecord) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO uploaded_invoices
                (source_id, external_id, paperless_task_id, invoice_issued_at, uploaded_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(&record.source_id)
        .bind(&record.external_id)
        .bind(&record.paperless_task_id)
        .bind(record.invoice_issued_at)
        .bind(record.uploaded_at)
        .execute(&self.pool)
        .await
        .context("Failed to insert upload record")?;

        Ok(())
    }

    pub async fn last_issued_date(&self, source_id: &str) -> anyhow::Result<Option<NaiveDate>> {
        let date: Option<NaiveDate> = sqlx::query_scalar(
            "SELECT invoice_issued_at FROM uploaded_invoices
             WHERE source_id = ?1
             ORDER BY invoice_issued_at DESC
             LIMIT 1",
        )
        .bind(source_id)
        .fetch_optional(&self.pool)
        .await
        .context("Failed to read last_issued_date")?;

        Ok(date)
    }
}
