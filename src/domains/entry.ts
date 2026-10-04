/** 仅承接用户主动提交；日期在写入时固化，AI 路径不得引用此写入模块。 */
import { isRecord, publicError } from '../bridge/shell'
import type { AppError } from '../bridge/shell'
import { database, transaction } from '../data/database'

export interface EntrySubmission {
  id: string
  content: string
  created_at: number
  local_date: string
  draft_seq: number
}

/** 同一提交对象贯穿重试；不得因为 IPC 结果不确定而生成新 UUID。 */
export function createSubmission(content: string, draftSeq: number): EntrySubmission {
  const createdAt = Date.now()
  const date = new Date(createdAt)
  const localDate = `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`
  return { id: crypto.randomUUID(), content, created_at: createdAt, local_date: localDate, draft_seq: draftSeq }
}

/** 按固定提交 ID 核实落盘内容；合法空结果表示该 ID 尚无记录，不推断查询失败。 */
async function committedContent(id: string): Promise<string | null> {
  const rows = await database().select<unknown>('SELECT content FROM entry WHERE id = $1', [id])
  if (!Array.isArray(rows)) throw { code: 'ENTRY_FORMAT', message: '记录保存状态无法读取。' }
  const row: unknown = rows[0]
  if (row === undefined) return null
  if (!isRecord(row) || typeof row.content !== 'string') throw { code: 'ENTRY_FORMAT', message: '记录保存状态格式不正确。' }
  return row.content
}

/** 新增记录与删除对应草稿在同一事务；重复 ID 不插入第二条，不清理更晚的草稿。 */
export async function submitEntry(submission: EntrySubmission): Promise<void> {
  let writeError: AppError | null = null
  try {
    await transaction([
      {
        query: `INSERT INTO entry (id, content, created_at, updated_at, local_date, deleted_at)
          VALUES ($1, $2, $3, $4, $5, NULL) ON CONFLICT(id) DO NOTHING`,
        values: [submission.id, submission.content, submission.created_at, submission.created_at, submission.local_date],
      },
      {
        query: `DELETE FROM draft WHERE id = $1 AND session_seq <= $2
          AND EXISTS (SELECT 1 FROM entry WHERE id = $3 AND content = $4)`,
        values: ['active', submission.draft_seq, submission.id, submission.content],
      },
    ])
  } catch (cause) {
    writeError = publicError(cause)
  }

  // 成功与异常均走同一核实路径；不因 IPC 报错直接判定记录未落盘。
  let committed: string | null
  try { committed = await committedContent(submission.id) }
  catch {
    // 核实失败也保留 Rust 返回的脱敏事务诊断，不展开插件原始查询异常。
    const transactionHint = writeError ? `事务提示：${writeError.message} ` : ''
    throw { code: 'ENTRY_STATE_UNCERTAIN', message: `保存结果暂时无法确认。${transactionHint}原文已锁定保留，请点保存重试，勿强制退出。` }
  }
  if (committed === submission.content) return
  if (committed === null && writeError) throw writeError
  throw { code: 'ENTRY_ID_CONFLICT', message: '记录未按预期保存，原文仍保留。请先复制原文，勿强制退出。' }
}
