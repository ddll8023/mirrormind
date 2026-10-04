/** 快速记录独立入口，不导入主窗口、路由或 AI 模块。 */
import { createApp } from 'vue'
import CaptureApp from './CaptureApp.vue'
import './style.css'

createApp(CaptureApp).mount('#app')
