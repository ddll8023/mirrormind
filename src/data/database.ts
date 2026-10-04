/** 使用 Rust 已注册的 SQL 插件连接，禁止默认 load 建库；多语句写入走薄事务。 */
import { invoke } from '@tauri-apps/api/core'
import Database from '@tauri-apps/plugin-sql'
import type { StorageInfo } from '../bridge/shell'
import { publicError } from '../bridge/shell'

export type SqlValue = string | number | boolean | null
export interface SqlStatement { query: string; values: SqlValue[] }
let connection: Database | null = null

/** 只绑定 Rust 已就绪的便携数据库，初始化失败时不创建备用连接。 */
export function bindDatabase(storage: StorageInfo): void {
  if (!storage.ready || !storage.db_url) {
    throw storage.error ?? { code: 'DB_NOT_READY', message: '数据库尚未就绪。' }
  }
  // get 只构造绑定对象；连接与迁移已由 Rust 完成，不调用 load 或 close。
  connection = Database.get(storage.db_url)
}

/** 获取当前 WebView 的插件绑定，未初始化时明确拒绝读写。 */
export function database(): Database {
  if (!connection) throw { code: 'DB_NOT_READY', message: '数据库连接尚未绑定。' }
  return connection
}

/** Rust 只执行事务；语句、参数及记录规则均由调用它的 TS 业务模块决定。 */
export async function transaction(statements: SqlStatement[]): Promise<void> {
  try { await invoke<void>('db_transaction', { statements }) }
  catch (error) { throw publicError(error) }
}
