//! 在 Rust 声明业务表结构，复用 SQL 插件底层的 SQLx 迁移与版本账本。

use std::{future::Future, pin::Pin};
use sqlx::{migrate::{MigrationSource, Migrator}, SqlitePool};
use tauri_plugin_sql::{Migration, MigrationKind};
use crate::error::AppError;

#[derive(Debug)]
struct DeclaredMigrations(Vec<Migration>);

impl MigrationSource<'static> for DeclaredMigrations {
    /// 将 Rust 声明转换为 SQLx 迁移源，版本执行与事务仍由既有迁移器管理。
    fn resolve(self) -> Pin<Box<dyn Future<Output = Result<Vec<sqlx::migrate::Migration>, sqlx::error::BoxDynError>> + Send>> {
        Box::pin(async move {
            Ok(self.0.into_iter().map(|migration| {
                sqlx::migrate::Migration::new(
                    migration.version,
                    migration.description.into(),
                    migration.kind.into(),
                    migration.sql.into(),
                    false,
                )
            }).collect())
        })
    }
}

/// 对已指定便携路径的连接池应用迁移；失败时不注册前端数据库访问。
pub async fn apply(pool: &SqlitePool) -> Result<(), AppError> {
    let declared = DeclaredMigrations(vec![Migration {
        version: 1,
        description: "创建主动记录与草稿",
        kind: MigrationKind::Up,
        sql: "
            CREATE TABLE entry (
                id TEXT PRIMARY KEY NOT NULL,
                content TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                local_date TEXT NOT NULL,
                deleted_at INTEGER
            );
            CREATE TABLE draft (
                id TEXT PRIMARY KEY NOT NULL,
                content TEXT NOT NULL,
                updated_at INTEGER NOT NULL,
                session_seq INTEGER NOT NULL DEFAULT 0 CHECK (session_seq >= 0)
            );
        ",
    }]);
    let migrator = Migrator::new(declared).await
        .map_err(|error| AppError::new("MIGRATION_SOURCE", format!("迁移声明无法加载：{error}")))?;
    migrator.run(pool).await
        .map_err(|error| AppError::new("MIGRATION_FAILED", format!("数据库迁移失败，未删除已有数据：{error}")))
}
