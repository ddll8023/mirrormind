/** 管理唯一 active 草稿；序号阻止旧快照覆盖新内容，不在页面散落 SQL。 */
import { isRecord } from '../bridge/shell'
import { database } from '../data/database'

export interface DraftSnapshot { content: string; updated_at: number; session_seq: number }

/** 读取 active 草稿并校验字段；不存在返回空值，格式异常不覆盖输入。 */
export async function readDraft(): Promise<DraftSnapshot | null> {
  const rows = await database().select<unknown>(
    "SELECT content, updated_at, session_seq FROM draft WHERE id = $1", ['active'],
  )
  if (!Array.isArray(rows)) throw { code: 'DRAFT_FORMAT', message: '草稿读取结果格式不正确，未覆盖编辑器内容。' }
  const row: unknown = rows[0]
  if (row === undefined) return null
  if (!isRecord(row) || typeof row.content !== 'string' || typeof row.updated_at !== 'number'
    || typeof row.session_seq !== 'number' || !Number.isSafeInteger(row.session_seq) || row.session_seq < 0) {
    throw { code: 'DRAFT_FORMAT', message: '草稿字段格式不正确，未覆盖编辑器内容。' }
  }
  return { content: row.content, updated_at: row.updated_at, session_seq: row.session_seq }
}

/** 覆盖保存当前草稿快照，仅允许相同或更晚序号写入以阻止迟到请求。 */
export async function saveDraft(snapshot: DraftSnapshot): Promise<void> {
  await database().execute(
    `INSERT INTO draft (id, content, updated_at, session_seq) VALUES ($1, $2, $3, $4)
     ON CONFLICT(id) DO UPDATE SET content = excluded.content,
       updated_at = excluded.updated_at, session_seq = excluded.session_seq
     WHERE excluded.session_seq >= draft.session_seq`,
    ['active', snapshot.content, snapshot.updated_at, snapshot.session_seq],
  )
}
