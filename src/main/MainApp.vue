<!-- 阶段 1 状态窗口：展示初始化与快捷键错误，不承担记录浏览或 SQL 写入。 -->
<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { listen } from '@tauri-apps/api/event'
import type { UnlistenFn } from '@tauri-apps/api/event'
import { openCapture, publicError, readRuntime, requestExit, setPaused, setShortcut } from '../bridge/shell'
import type { AppError, RuntimeSnapshot } from '../bridge/shell'

const runtime = ref<RuntimeSnapshot | null>(null)
const shortcut = ref('')
const shortcutEdited = ref(false)
const busy = ref(false)
const loading = ref(true)
const actionError = ref<AppError | null>(null)
const captureState = computed(() => runtime.value?.capture_ready ? '草稿已载入，等待调起'
  : runtime.value?.storage.error || runtime.value?.capture_error ? '初始化失败' : '正在初始化')
const hotkeyState = computed(() => runtime.value?.hotkey.paused ? '已暂停'
  : runtime.value?.hotkey.registered ? '已注册' : '未注册，请检查错误或换键')
let revision = 0
let disposed = false
let unlisten: UnlistenFn | undefined

/** 状态广播只是刷新提示，首次打开与重新聚焦均主动查询；旧响应不能覆盖新响应。 */
async function refresh(): Promise<void> {
  const current = ++revision
  try {
    const snapshot = await readRuntime()
    if (disposed || current !== revision) return
    runtime.value = snapshot
    if (!shortcutEdited.value) shortcut.value = snapshot.hotkey.shortcut
  } catch (cause) {
    if (!disposed && current === revision) actionError.value = publicError(cause)
  } finally { if (current === revision) loading.value = false }
}

/** 串行执行用户系统操作，统一显示错误并刷新状态，避免重复点击。 */
async function operate(operation: () => Promise<unknown>): Promise<void> {
  if (busy.value) return
  busy.value = true
  actionError.value = null
  try { await operation() }
  catch (cause) { actionError.value = publicError(cause) }
  finally { await refresh(); busy.value = false }
}

/** 提交组合键，仅在注册成功后更新输入框与实际配置。 */
async function applyShortcut(): Promise<void> {
  await operate(async () => {
    const updated = await setShortcut(shortcut.value)
    shortcut.value = updated.shortcut
    shortcutEdited.value = false
  })
}

/** 仅填入候选组合键，不未经用户确认自动注册。 */
function chooseCandidate(value: string): void { shortcut.value = value; shortcutEdited.value = true }
/** 主窗口重新聚焦时主动查询状态，补偿隐藏期间丢失的广播。 */
function onFocus(): void { void refresh() }

onMounted(() => {
  window.addEventListener('focus', onFocus)
  void refresh()
  void listen('runtime-changed', onFocus).then((cleanup) => {
    if (disposed) cleanup()
    else unlisten = cleanup
  }).catch((cause: unknown) => { actionError.value = publicError(cause) })
})

onBeforeUnmount(() => {
  disposed = true
  revision += 1
  window.removeEventListener('focus', onFocus)
  unlisten?.()
})
</script>

<template>
  <main class="mx-auto max-w-3xl px-8 py-8" :aria-busy="busy || loading">
    <header class="mb-7 border-l-4 border-accent pl-4">
      <p class="mb-1 text-xs tracking-widest text-accent">MIRRORMIND / 阶段 1</p>
      <h1 class="text-2xl font-semibold">先让记录可靠落下</h1>
      <p class="mt-2 text-sm leading-6 text-slate-600">这里只显示骨架状态。记录浏览、搜索与 AI 尚未实现。</p>
    </header>

    <section aria-labelledby="runtime-title" class="mb-6 rounded-lg border border-slate-200 bg-white p-5">
      <div class="mb-4 flex items-center justify-between gap-3">
        <h2 id="runtime-title" class="font-semibold">运行状态</h2>
        <button type="button" class="text-sm text-accent underline underline-offset-4" :disabled="busy" @click="refresh">刷新状态</button>
      </div>
      <dl class="grid grid-cols-[7rem_1fr] gap-x-4 gap-y-3 text-sm">
        <dt class="text-slate-500">快速记录</dt><dd>{{ captureState }}</dd>
        <dt class="text-slate-500">数据库</dt><dd>{{ runtime?.storage.ready ? '已连接 · WAL' : '尚未就绪' }}</dd>
        <dt class="text-slate-500">数据文件</dt><dd class="break-all font-mono text-xs leading-6">{{ runtime?.storage.database_path ?? '正在解析便携路径' }}</dd>
        <dt class="text-slate-500">持久性初值</dt><dd>{{ runtime?.storage.synchronous ?? 'FULL' }} · 尚未实测校准</dd>
        <dt class="text-slate-500">快捷键</dt><dd>{{ hotkeyState }}</dd>
      </dl>
      <p v-if="runtime?.storage.error" class="mt-4 text-sm text-red-800" role="alert">{{ runtime.storage.error.message }}</p>
      <p v-if="runtime?.capture_error" class="mt-4 text-sm text-red-800" role="alert">快速记录初始化失败：{{ runtime.capture_error.message }}</p>
      <p v-if="runtime?.shell_error" class="mt-4 text-sm text-amber-800" role="status">最近壳层提示：{{ runtime.shell_error.message }}</p>
      <button type="button" class="mt-5 rounded-md bg-accent px-4 py-2 text-sm font-semibold text-white"
        :disabled="busy || !runtime?.capture_ready" @click="operate(openCapture)">打开快速记录</button>
    </section>

    <section aria-labelledby="shortcut-title" class="rounded-lg border border-slate-200 bg-white p-5">
      <h2 id="shortcut-title" class="mb-4 font-semibold">快捷键</h2>
      <form @submit.prevent="applyShortcut">
        <label for="shortcut" class="mb-2 block text-sm text-slate-600">组合键</label>
        <div class="flex gap-3">
          <input id="shortcut" v-model="shortcut" class="min-w-0 flex-1 rounded-md border border-slate-300 px-3 py-2 font-mono text-sm"
            :disabled="busy" aria-describedby="shortcut-help" @input="shortcutEdited = true" />
          <button type="submit" class="rounded-md border border-accent px-4 py-2 text-sm text-accent" :disabled="busy || !shortcut.trim()">应用换键</button>
        </div>
        <p id="shortcut-help" class="mt-2 text-xs leading-5 text-slate-500">Windows 用 Control；macOS 用 Super 表示 Command。注册失败无法判断被哪个程序占用。</p>
        <div class="mt-3 flex flex-wrap gap-3 text-xs">
          <button type="button" class="underline underline-offset-4" :disabled="busy" @click="chooseCandidate('Control+Alt+Space')">填入 Control+Alt+Space</button>
          <button type="button" class="underline underline-offset-4" :disabled="busy" @click="chooseCandidate('Super+Alt+Space')">填入 Super+Alt+Space</button>
        </div>
      </form>
      <p v-if="runtime?.hotkey.error" class="mt-4 text-sm text-red-800" role="alert">{{ runtime.hotkey.error.message }}</p>
      <button type="button" class="mt-4 rounded-md border border-slate-300 px-4 py-2 text-sm" :disabled="busy || !runtime"
        @click="operate(() => setPaused(!runtime?.hotkey.paused))">{{ runtime?.hotkey.paused ? '恢复快捷键' : '暂停快捷键' }}</button>
      <p class="mt-3 text-xs text-slate-500">换键与暂停仅对本次运行生效；配置持久化留到下一阶段。</p>
    </section>

    <p v-if="actionError" class="mt-5 text-sm text-red-800" role="alert">{{ actionError.message }}</p>
    <footer class="mt-6 flex items-start justify-between gap-5 border-t border-slate-200 pt-5">
      <p class="max-w-md text-xs leading-5 text-slate-500">关闭主窗口只收起到托盘。焦点归还、中文输入、发行版与 macOS 行为仍待实测；未完成双平台验证。</p>
      <button type="button" class="shrink-0 rounded-md border border-slate-300 px-3 py-2 text-xs" :disabled="busy" @click="operate(requestExit)">保存草稿并退出</button>
    </footer>
  </main>
</template>
