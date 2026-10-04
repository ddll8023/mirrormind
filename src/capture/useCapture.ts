/** 管理快速记录的单写入队列、输入法边界与保存确认，不依赖隐藏页面定时器调度。 */
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from 'vue'
import { listen } from '@tauri-apps/api/event'
import type { UnlistenFn } from '@tauri-apps/api/event'
import type { ShellAction } from '../bridge/shell'
import { finishShellAction, markCaptureReady, publicError, readRuntime, requestHide } from '../bridge/shell'
import type { AppError } from '../bridge/shell'
import { bindDatabase } from '../data/database'
import { readDraft, saveDraft } from '../domains/draft'
import { createSubmission, submitEntry } from '../domains/entry'
import type { EntrySubmission } from '../domains/entry'

// 仅为阶段 1 的保守初值，后续按写入次数、输入延迟和丢失窗口实测校准。
const INPUT_PAUSE_MS = 500
const MAX_WAIT_MS = 2_000

/** 为单个预创建编辑器持有草稿、写入队列及生命周期监听，不创建跨窗口业务状态。 */
export function useCapture() {
  const textarea = ref<HTMLTextAreaElement | null>(null)
  const content = ref('')
  const ready = ref(false)
  const dirty = ref(false)
  const saving = ref(false)
  const committing = ref(false)
  const closing = ref(false)
  const uncertain = ref(false)
  const error = ref<AppError | null>(null)
  const locked = computed(() => !ready.value || committing.value || closing.value || uncertain.value)
  const status = computed(() => error.value ? '内容保留，请处理错误'
    : committing.value ? '正在提交' : saving.value ? '正在保存草稿'
      : dirty.value ? '尚有未保存输入' : ready.value ? '草稿已同步' : '正在载入草稿')
  let sequence = 0
  let selection = { start: 0, end: 0 }
  let composing = false
  let activationSynced = false
  let disposed = false
  let idleTimer: ReturnType<typeof setTimeout> | undefined
  let maxTimer: ReturnType<typeof setTimeout> | undefined
  let queue: Promise<void> = Promise.resolve()
  let attempt: EntrySubmission | null = null
  const unlisteners: UnlistenFn[] = []

  /** 自动保存、同步、提交和退出共用串行队列；失败不污染后续任务。 */
  function serialize<T>(operation: () => Promise<T>): Promise<T> {
    const result = queue.then(operation)
    queue = result.then(() => undefined, () => undefined)
    return result
  }

  /** 在交互边界呈现公开错误，不输出或丢弃编辑器原文。 */
  function showError(cause: unknown): void { error.value = publicError(cause) }

  /** 取消本轮输入触发的保存计时，隐藏和退出不留下周期性页面工作。 */
  function clearTimers(): void {
    clearTimeout(idleTimer)
    clearTimeout(maxTimer)
    idleTimer = undefined
    maxTimer = undefined
  }

  /** 在当前窗口内保存光标范围，供再次调起恢复，不监听其他应用输入。 */
  function rememberSelection(): void {
    const input = textarea.value
    if (input) selection = { start: input.selectionStart, end: input.selectionEnd }
  }

  /** 等待 DOM 更新后恢复输入焦点与光标，组合输入期间不干预输入法。 */
  async function focusEditor(): Promise<void> {
    if (disposed || composing || !ready.value) return
    await nextTick()
    const input = textarea.value
    if (!input) return
    input.focus({ preventScroll: true })
    input.setSelectionRange(Math.min(selection.start, content.value.length), Math.min(selection.end, content.value.length))
  }

  /** 保存快照而不是队列创建时的旧值；更晚输入保留 dirty，不被旧完成回调清掉。 */
  async function flushDraft(): Promise<void> {
    if (!ready.value || !dirty.value || composing) return
    clearTimers()
    const snapshot = { content: content.value, updated_at: Date.now(), session_seq: sequence }
    saving.value = true
    try {
      await saveDraft(snapshot)
      if (sequence === snapshot.session_seq) { dirty.value = false; error.value = null }
    } finally {
      saving.value = false
      if (dirty.value && sequence !== snapshot.session_seq && !composing && !locked.value) scheduleSave()
    }
  }

  /** 输入停顿时保存，并为同一轮连续输入设最大等待；不使用固定周期循环。 */
  function scheduleSave(): void {
    if (composing || !ready.value || locked.value) return
    clearTimeout(idleTimer)
    idleTimer = setTimeout(() => { void serialize(flushDraft).catch(showError) }, INPUT_PAUSE_MS)
    if (maxTimer === undefined) {
      maxTimer = setTimeout(() => { void serialize(flushDraft).catch(showError) }, MAX_WAIT_MS)
    }
  }

  /** 仅在落盘核实成功后清空对应输入、提交上下文与旧保存计时。 */
  function clearCommittedInput(): void {
    content.value = ''
    dirty.value = false
    sequence += 1
    selection = { start: 0, end: 0 }
    attempt = null
    uncertain.value = false
    error.value = null
    clearTimers()
  }

  /** 保存与退出共用提交确认；不确定结果保留 UUID，确认未提交后才允许重新编辑。 */
  async function confirmSubmission(submission: EntrySubmission): Promise<void> {
    try {
      await submitEntry(submission)
      clearCommittedInput()
    } catch (cause) {
      const failure = publicError(cause)
      uncertain.value = failure.code === 'ENTRY_STATE_UNCERTAIN' || failure.code === 'ENTRY_ID_CONFLICT'
      if (!uncertain.value) attempt = null
      throw failure
    }
  }

  /** 不确定的提交必须先按原 UUID 核实，不能重新保存为草稿后退出而制造重复记录。 */
  async function finishAction(action: ShellAction): Promise<void> {
    if (composing || disposed) return
    closing.value = true
    clearTimers()
    try {
      if (uncertain.value && attempt) await confirmSubmission(attempt)
      await flushDraft()
      const accepted = await finishShellAction(action.id)
      if (!accepted) {
        const latest = await readRuntime()
        if (latest.pending_action) await finishAction(latest.pending_action)
      }
    } finally { closing.value = false }
  }

  /** 查询原生动作后再同步草稿；异步读取不能覆盖用户已经输入或正在组合的文本。 */
  async function synchronize(): Promise<void> {
    if (!ready.value || disposed) return
    await serialize(async () => {
      const snapshot = await readRuntime()
      if (snapshot.pending_action) { await finishAction(snapshot.pending_action); return }
      const startedAt = sequence
      if (!dirty.value && !composing && !uncertain.value) {
        const draft = await readDraft()
        if (sequence === startedAt && !dirty.value && !composing && !uncertain.value) {
          content.value = draft?.content ?? ''
          sequence = Math.max(sequence, draft?.session_seq ?? 0)
        }
      }
      activationSynced = true
      await focusEditor()
    })
  }

  /** DOM 焦点或壳层提示到来时主动同步，不把提示本身作为可靠动作载体。 */
  function onFocus(): void { void synchronize().catch(showError) }
  /** 页面重新可见时补充同步机会，不依赖隐藏页面定时器。 */
  function onVisibility(): void { if (!document.hidden) onFocus() }
  /** 失焦保留光标并排队保存草稿，不把失焦等同于销毁或退出。 */
  function onBlur(): void {
    rememberSelection()
    activationSynced = false
    if (ready.value) void serialize(flushDraft).catch(showError)
  }

  /** 记录当前编辑器的主动输入与修订序号，脏状态保护输入不被异步读取覆盖。 */
  function onInput(event: Event): void {
    const input = event.target
    if (!(input instanceof HTMLTextAreaElement) || locked.value) return
    content.value = input.value
    sequence += 1
    dirty.value = true
    rememberSelection()
    if (!composing) scheduleSave()
    // DOM 唤醒信号遗漏时，第一次主动输入仍可查询待办；dirty 保护新输入不被草稿覆盖。
    if (!activationSynced) {
      activationSynced = true
      void synchronize().catch(showError)
    }
  }

  /** 输入法开始组词时停止自动保存计时，避免把组合过程当作已完成输入。 */
  function onCompositionStart(): void { composing = true; clearTimers() }
  /** 接收输入法最终文本后恢复保存并检查待隐藏或退出动作。 */
  function onCompositionEnd(event: CompositionEvent): void {
    composing = false
    onInput(event)
    scheduleSave()
    void synchronize().catch(showError)
  }

  /** 用户主动收起时请求动作 ID，草稿保存成功后才确认隐藏。 */
  async function hide(): Promise<void> {
    if (!ready.value || composing || closing.value || committing.value) return
    try {
      const action = await requestHide()
      if (action) await serialize(() => finishAction(action))
    } catch (cause) { showError(cause) }
  }

  /** 空内容只保留草稿并隐藏；非空提交固定 UUID，只有持久化确认后才清空输入。 */
  async function submit(): Promise<void> {
    if (!ready.value || composing || committing.value || closing.value) return
    committing.value = true
    clearTimers()
    try {
      await serialize(async () => {
        if (!attempt) {
          await flushDraft()
          if (!content.value.trim()) return
          attempt = createSubmission(content.value, sequence)
        }
        await confirmSubmission(attempt)
      })
      const action = await requestHide()
      if (action) await serialize(() => finishAction(action))
    } catch (cause) { showError(cause) }
    finally { committing.value = false }
  }

  /** 仅处理非输入法候选状态的窗口快捷键，避免 Esc 取消候选或 Enter 选词误触操作。 */
  function onKeydown(event: KeyboardEvent): void {
    if (composing || event.isComposing || event.keyCode === 229) return
    if (event.key === 'Escape') { event.preventDefault(); void hide() }
    else if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) { event.preventDefault(); void submit() }
  }

  onMounted(() => {
    window.addEventListener('focus', onFocus)
    window.addEventListener('blur', onBlur)
    document.addEventListener('visibilitychange', onVisibility)
    // 通知注册不阻塞隐藏页面的首次载入；启动和调起的关键通路使用主动查询与 DOM 信号。
    void listen('shell-intent', onFocus).then((unlisten) => {
      if (disposed) unlisten()
      else unlisteners.push(unlisten)
    }).catch(showError)
    void (async () => {
      try {
        const snapshot = await readRuntime()
        bindDatabase(snapshot.storage)
        const draft = await readDraft()
        if (disposed) return
        content.value = draft?.content ?? ''
        sequence = draft?.session_seq ?? 0
        selection = { start: content.value.length, end: content.value.length }
        await markCaptureReady()
        ready.value = true
      } catch (cause) {
        showError(cause)
        try { await markCaptureReady(error.value?.message ?? '快速记录初始化失败。') }
        catch { /* 主窗口仍显示未就绪状态；不伪造可输入确认。 */ }
      }
      if (ready.value && !disposed) {
        // 启动查询失败不能把已经可输入的编辑器标记为安全退出态。
        try {
          const latest = await readRuntime()
          const action = latest.pending_action
          if (action) await serialize(() => finishAction(action))
        } catch (cause) { showError(cause) }
      }
    })()
  })

  onBeforeUnmount(() => {
    disposed = true
    clearTimers()
    window.removeEventListener('focus', onFocus)
    window.removeEventListener('blur', onBlur)
    document.removeEventListener('visibilitychange', onVisibility)
    for (const unlisten of unlisteners) unlisten()
    // 不依赖卸载或 unload 保存；正常隐藏与退出已先完成保存确认握手。
  })

  return { textarea, content, ready, locked, saving, committing, closing, error, status,
    onInput, onKeydown, onCompositionStart, onCompositionEnd, rememberSelection, submit, hide }
}
