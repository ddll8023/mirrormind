/** 主窗口独立入口，本轮仅提供状态与壳层控制。 */
import { createApp } from 'vue'
import MainApp from './MainApp.vue'
import './style.css'

createApp(MainApp).mount('#app')
