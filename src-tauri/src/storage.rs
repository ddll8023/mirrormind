//! 初始化便携 SQLite 连接并执行薄事务；业务 SQL 与提交规则由 TS 提供。

use std::{fs, path::{Path, PathBuf}, sync::Mutex, time::Duration};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous}, SqlitePool};
use tauri::{AppHandle, Manager, WebviewWindow};
use tauri_plugin_sql::{DbInstances, DbPool};
use crate::{error::AppError, migrations};

#[derive(Clone, Serialize)]
pub struct StorageInfo {
    pub ready: bool,
    pub data_dir: Option<String>,
    pub database_path: Option<String>,
    pub db_url: Option<String>,
    pub synchronous: &'static str,
    pub error: Option<AppError>,
}

pub struct StorageState {
    // 仅初始化任务写入；窗口通过状态命令读取，不持有第二套连接池。
    info: Mutex<StorageInfo>,
}

impl Default for StorageState {
    /// 创建尚未就绪的存储快照，不在构造时访问目录或数据库。
    fn default() -> Self {
        Self { info: Mutex::new(StorageInfo {
            ready: false, data_dir: None, database_path: None, db_url: None,
            synchronous: "FULL", error: None,
        }) }
    }
}

impl StorageState {
    /// 在短临界区克隆初始化状态，调用者不持有存储状态锁。
    pub fn snapshot(&self) -> Result<StorageInfo, AppError> {
        self.info.lock().map(|info| info.clone())
            .map_err(|_| AppError::new("STATE_LOCK", "存储状态不可读取，请重启应用。"))
    }

    /// 由初始化任务整体替换快照，避免页面读到部分更新的连接信息。
    fn replace(&self, info: StorageInfo) -> Result<(), AppError> {
        *self.info.lock().map_err(|_| AppError::new("STATE_LOCK", "存储状态不可更新。"))? = info;
        Ok(())
    }
}

/// 开发态固定使用仓库 data；发行态使用 exe 或 .app 所在便携目录。
fn data_directory() -> Result<PathBuf, AppError> {
    if cfg!(debug_assertions) {
        return Path::new(env!("CARGO_MANIFEST_DIR")).parent()
            .map(|root| root.join("data"))
            .ok_or_else(|| AppError::new("PORTABLE_PATH", "开发目录无法解析。"));
    }
    let executable = std::env::current_exe()
        .map_err(|error| AppError::new("PORTABLE_PATH", format!("运行文件路径无法解析：{error}")))?;
    let parent = executable.parent()
        .ok_or_else(|| AppError::new("PORTABLE_PATH", "运行文件没有父目录。"))?;
    #[cfg(target_os = "macos")]
    {
        if executable.components().any(|part| part.as_os_str() == "AppTranslocation") {
            return Err(AppError::new("PORTABLE_TRANSLOCATED", "应用处于 macOS 隔离转移目录，无法确定原始便携目录。请从普通可写目录启动；不会回退到系统数据目录。"));
        }
        if parent.file_name().is_some_and(|name| name == "MacOS") {
            if let Some(contents) = parent.parent().filter(|path| path.file_name().is_some_and(|name| name == "Contents")) {
                if let Some(bundle) = contents.parent().filter(|path| path.extension().is_some_and(|extension| extension == "app")) {
                    return bundle.parent().map(|directory| directory.join("data"))
                        .ok_or_else(|| AppError::new("PORTABLE_PATH", ".app 的便携父目录无法解析。"));
                }
            }
        }
    }
    Ok(parent.join("data"))
}

/// 检查目录可写性；只创建并移除本进程自己的探测文件，不清理已有数据。
fn prepare_directory() -> Result<PathBuf, AppError> {
    let directory = data_directory()?;
    fs::create_dir_all(&directory)
        .map_err(|error| AppError::new("PORTABLE_NOT_WRITABLE", format!("无法创建便携数据目录 {}：{error}", directory.display())))?;
    let probe = directory.join(format!(".mirrormind-write-check-{}", std::process::id()));
    let file = fs::OpenOptions::new().write(true).create_new(true).open(&probe)
        .map_err(|error| AppError::new("PORTABLE_NOT_WRITABLE", format!("便携数据目录不可写 {}：{error}", directory.display())))?;
    drop(file);
    fs::remove_file(&probe)
        .map_err(|error| AppError::new("PORTABLE_NOT_WRITABLE", format!("便携目录内的写入探测文件无法移除：{error}")))?;
    Ok(directory)
}

/// 建立带连接级 PRAGMA 的唯一连接池；初始化错误保留在主窗口可查询的状态中。
pub async fn initialize(app: &AppHandle) -> Result<(), AppError> {
    let prepared = tauri::async_runtime::spawn_blocking(prepare_directory).await
        .map_err(|error| AppError::new("STORAGE_INIT", format!("存储初始化任务失败：{error}")))
        .and_then(|result| result);
    let directory = match prepared {
        Ok(directory) => directory,
        Err(error) => {
            let mut info = app.state::<StorageState>().snapshot()?;
            info.error = Some(error.clone());
            app.state::<StorageState>().replace(info)?;
            return Err(error);
        }
    };
    let database_path = directory.join("mirrormind.sqlite3");
    let path_text = database_path.to_str()
        .ok_or_else(|| AppError::new("PORTABLE_PATH", "数据库路径无法编码为 UTF-8。"))?;
    let db_url = format!("sqlite:{path_text}");
    let mut info = StorageInfo {
        ready: false,
        data_dir: Some(directory.to_string_lossy().into_owned()),
        database_path: Some(path_text.to_owned()), db_url: Some(db_url.clone()),
        synchronous: "FULL", error: None,
    };
    app.state::<StorageState>().replace(info.clone())?;
    let options = SqliteConnectOptions::new().filename(&database_path)
        .create_if_missing(true).journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Full).busy_timeout(Duration::from_secs(5));
    let result = async {
        // 单连接足够本轮验证，也使插件查询与薄事务按连接获取顺序串行。
        let pool = SqlitePoolOptions::new().max_connections(1).min_connections(1)
            .connect_with(options).await
            .map_err(|error| AppError::new("DB_OPEN", format!("便携数据库无法打开：{error}")))?;
        if let Err(error) = migrations::apply(&pool).await {
            pool.close().await;
            return Err(error);
        }
        let instances = app.state::<DbInstances>();
        instances.0.write().await.insert(db_url, DbPool::Sqlite(pool));
        Ok(())
    }.await;
    match result {
        Ok(()) => { info.ready = true; app.state::<StorageState>().replace(info) }
        Err(error) => {
            info.error = Some(error.clone());
            app.state::<StorageState>().replace(info)?;
            Err(error)
        }
    }
}

/// 只提取错误类别与数字 SQLite 扩展码；禁止读取原始错误消息、SQL 正文或参数。
fn sql_failure_details(error: &sqlx::Error) -> String {
    let sqlite_code = error.as_database_error()
        .and_then(|database_error| database_error.code())
        .and_then(|code| code.parse::<u32>().ok());
    if let Some(code) = sqlite_code {
        // SQLite 扩展码的低八位是主错误码；显示时仍保留完整扩展码。
        let category = match code & 0xff {
            5 | 6 => "数据库锁冲突",
            8 => "数据库只读",
            10 => "磁盘读写失败",
            11 | 26 => "数据库损坏或格式不符",
            13 => "磁盘空间不足",
            14 => "数据库文件无法打开",
            19 => "数据约束冲突",
            _ => "SQLite 错误",
        };
        return format!("{category}，SQLite 错误码 {code}");
    }
    match error {
        sqlx::Error::Io(_) => "数据库 I/O 故障",
        sqlx::Error::PoolTimedOut => "数据库连接等待超时",
        sqlx::Error::PoolClosed => "数据库连接已关闭",
        sqlx::Error::WorkerCrashed => "数据库工作线程故障",
        sqlx::Error::Protocol(_) => "数据库协议故障",
        sqlx::Error::Encode(_) => "SQL 参数编码失败",
        sqlx::Error::Decode(_) | sqlx::Error::ColumnDecode { .. } => "SQL 结果解码失败",
        sqlx::Error::BeginFailed => "事务开始被驱动拒绝",
        _ => "SQLx 未分类错误",
    }.to_owned()
}

#[derive(Deserialize)]
pub struct SqlStatement {
    query: String,
    values: Vec<Value>,
}

/// 仅接受本地 capture 的参数化语句；在插件同一连接上提交或回滚，不解释业务表。
/// 提交阶段失败可能存在结果不确定性，前端必须保留提交 ID 与原文后再核实。
#[tauri::command]
pub async fn db_transaction(
    window: WebviewWindow,
    app: AppHandle,
    statements: Vec<SqlStatement>,
) -> Result<(), AppError> {
    if window.label() != "capture" {
        return Err(AppError::new("FORBIDDEN", "该窗口不能发起数据库事务。"));
    }
    if statements.is_empty() || statements.len() > 8 {
        return Err(AppError::new("INVALID_SQL_BATCH", "事务语句数量不合法。"));
    }
    for statement in &statements {
        if statement.query.trim().is_empty() || statement.query.len() > 32_768 || statement.values.len() > 64 {
            return Err(AppError::new("INVALID_SQL_BATCH", "事务语句或参数数量不合法。"));
        }
        for value in &statement.values {
            if !value.is_null() && !value.is_boolean() && !value.is_number() && !value.is_string() {
                return Err(AppError::new("INVALID_SQL_VALUE", "SQL 参数只支持字符串、数字、布尔值与空值。"));
            }
            if value.as_str().is_some_and(|text| text.len() > 4_194_304) {
                return Err(AppError::new("INVALID_SQL_VALUE", "单个 SQL 文本参数超过 4 MiB，内容未丢弃。"));
            }
        }
    }
    let info = app.state::<StorageState>().snapshot()?;
    let url = info.db_url.filter(|_| info.ready)
        .ok_or_else(|| AppError::new("DB_NOT_READY", "数据库尚未就绪。"))?;
    let pool: SqlitePool = {
        let instances = app.state::<DbInstances>();
        let loaded = instances.0.read().await;
        match loaded.get(&url) {
            Some(DbPool::Sqlite(pool)) => pool.clone(),
            _ => return Err(AppError::new("DB_NOT_READY", "数据库连接尚未注册。")),
        }
    };
    let mut transaction = pool.begin().await
        .map_err(|error| AppError::new("DB_TRANSACTION", format!("事务开始失败（{}），原文仍保留。", sql_failure_details(&error))))?;
    for (index, statement) in statements.into_iter().enumerate() {
        let mut query = sqlx::query(&statement.query);
        for value in statement.values {
            query = match value {
                Value::Null => query.bind(None::<String>),
                Value::String(text) => query.bind(text),
                Value::Bool(boolean) => query.bind(boolean),
                Value::Number(number) => {
                    if let Some(integer) = number.as_i64() { query.bind(integer) }
                    else if let Some(float) = number.as_f64() { query.bind(float) }
                    else { return Err(AppError::new("INVALID_SQL_VALUE", "数字参数无法安全绑定。")); }
                }
                _ => return Err(AppError::new("INVALID_SQL_VALUE", "SQL 参数类型不合法。")),
            };
        }
        if let Err(error) = query.execute(&mut *transaction).await {
            let execute_details = sql_failure_details(&error);
            if let Err(rollback_error) = transaction.rollback().await {
                return Err(AppError::new("DB_ROLLBACK_UNCERTAIN", format!(
                    "执行第 {} 条语句失败（{execute_details}）；事务回滚未获确认（{}）。请保留原文并重试核实。",
                    index + 1, sql_failure_details(&rollback_error),
                )));
            }
            return Err(AppError::new("DB_TRANSACTION", format!(
                "执行第 {} 条语句失败（{execute_details}），已回滚；原文仍保留，请重试。", index + 1,
            )));
        }
    }
    transaction.commit().await
        .map_err(|error| AppError::new("DB_COMMIT_UNCERTAIN", format!(
            "事务提交未获确认（{}）。请保留原文并重试核实。", sql_failure_details(&error),
        )))
}
