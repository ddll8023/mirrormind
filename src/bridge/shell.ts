/** 定义双窗口的本地 IPC 契约；系统命令不采用 HTTP 响应壳。 */
import { invoke } from '@tauri-apps/api/core'

export interface AppError { code: string; message: string }
export interface ShellAction { id: number; kind: 'hide' | 'exit' }
export interface StorageInfo {
  ready: boolean
  data_dir: string | null
  database_path: string | null
  db_url: string | null
  synchronous: string
  error: AppError | null
}
export interface HotkeyInfo {
  shortcut: string
  paused: boolean
  registered: boolean
  error: AppError | null
}
export interface RuntimeSnapshot {
  capture_ready: boolean
  capture_error: AppError | null
  pending_action: ShellAction | null
  shell_error: AppError | null
  storage: StorageInfo
  hotkey: HotkeyInfo
}

/** 将未知 IPC 值收窄为普通对象，拒绝空值与数组。 */
export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/** 插件异常可能夹带 SQL 信息，未识别的错误只呈现通用提示，不输出用户正文。 */
export function publicError(value: unknown): AppError {
  if (isRecord(value) && typeof value.code === 'string' && typeof value.message === 'string') {
    return { code: value.code, message: value.message }
  }
  return { code: 'LOCAL_OPERATION_FAILED', message: '本地操作失败，内容未丢弃。请重试；若持续失败，请检查便携目录权限。' }
}

/** 主动读取运行快照，不把状态广播当作可靠存储。 */
export const readRuntime = () => invoke<RuntimeSnapshot>('runtime_status')
/** capture 上报草稿已载入，或独立保留初始化失败原因。 */
export const markCaptureReady = (failure: string | null = null) => invoke<void>('capture_ready', { failure })
/** 调起已预创建的快速记录窗口，由 Rust 定位并聚焦。 */
export const openCapture = () => invoke<void>('open_capture')
/** 请求保留草稿后隐藏，返回需确认的动作而非立即隐藏。 */
export const requestHide = () => invoke<ShellAction | null>('request_hide')
/** 请求正常退出，由 Rust 唤醒 capture 完成保存确认。 */
export const requestExit = () => invoke<void>('request_exit')
/** 保存完成后确认指定动作；旧动作不会隐藏窗口或退出应用。 */
export const finishShellAction = (actionId: number) => invoke<boolean>('finish_shell_action', { actionId })
/** 主窗口修改本次运行的组合键，注册失败保持旧键。 */
export const setShortcut = (shortcut: string) => invoke<HotkeyInfo>('set_shortcut', { shortcut })
/** 主窗口暂停或恢复快捷键，不影响数据与草稿。 */
export const setPaused = (paused: boolean) => invoke<HotkeyInfo>('set_paused', { paused })
