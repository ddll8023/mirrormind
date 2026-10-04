<!-- 轻量快速记录窗口：普通 textarea 与可见保存反馈，不执行网络或 AI 调用。 -->
<script setup lang="ts">
import { useCapture } from './useCapture'

const { textarea, content, ready, locked, saving, committing, closing, error, status,
  onInput, onKeydown, onCompositionStart, onCompositionEnd, rememberSelection, submit, hide } = useCapture()
</script>

<template>
  <main class="flex h-full flex-col border border-slate-300 bg-paper p-4">
    <header class="mb-3 flex cursor-move items-center justify-between gap-4" data-tauri-drag-region>
      <h1 id="capture-label" class="border-l-4 border-accent pl-3 text-sm font-semibold tracking-wide" data-tauri-drag-region>
        快速记录
      </h1>
      <span class="text-xs text-slate-500" data-tauri-drag-region>主动写下，不后台采集</span>
    </header>
    <textarea
      ref="textarea"
      :value="content"
      :readonly="locked"
      :aria-busy="saving || committing"
      aria-labelledby="capture-label"
      aria-describedby="capture-help capture-status"
      class="min-h-16 flex-1 rounded-md border border-slate-200 bg-white p-3 text-base leading-7 outline-offset-0"
      placeholder="想法、任务、决定、问题……"
      spellcheck="false"
      @input="onInput"
      @keydown="onKeydown"
      @compositionstart="onCompositionStart"
      @compositionend="onCompositionEnd"
      @select="rememberSelection"
      @keyup="rememberSelection"
      @click="rememberSelection"
    />
    <p v-if="error" class="mt-2 max-h-14 overflow-y-auto text-xs text-red-800" role="alert">{{ error.message }}</p>
    <footer class="mt-3 flex items-center justify-between gap-3">
      <div class="min-w-0">
        <p id="capture-status" class="text-xs text-slate-600" role="status">{{ status }}</p>
        <p id="capture-help" class="mt-1 text-xs text-slate-500">Ctrl / Cmd + Enter 保存 · Esc 保留并收起</p>
      </div>
      <div class="flex shrink-0 gap-2">
        <button type="button" class="rounded-md border border-slate-300 px-3 py-2 text-xs"
          :disabled="!ready || committing || closing" @click="hide">保留并收起</button>
        <button type="button" class="rounded-md bg-accent px-3 py-2 text-xs font-semibold text-white"
          :disabled="!ready || committing || closing" @click="submit">{{ committing ? '保存中' : '保存记录' }}</button>
      </div>
    </footer>
  </main>
</template>
