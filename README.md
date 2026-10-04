# MirrorMind

自用的跨平台主动式记录工具：程序长期后台常驻，用全局快捷键随时调起轻量记录窗口，主动写下想法、任务、决定、问题与灵感，内容按日期归集。

## 项目状态

阶段 1：最小骨架与基础通路。Windows 本机已形成基本记录闭环，双平台整体目标尚未完成。

- 已实现：快速记录窗口与草稿自动保存、托盘常驻、单实例、全局快捷键、便携目录内 SQLite 落盘、快捷键换键与暂停入口、错误可见化。
- 未实现：记录浏览与编辑、搜索、AI 总结、导出与备份、完整设置页。
- 已有产物：按便携方式构建的 `src-tauri/target/release/mirrormind.exe`（不产出安装包）。

## 设计边界

- **只做主动记录**：不存在被动监听、键盘或输入法采集、读取其他应用输入框、截屏识别等能力，也不计划引入。
- **本地优先**：无账号、无云服务、无遥测；捕获保存路径中不含 AI 调用与网络请求。
- **数据与配置随程序走**：数据固定在可执行文件同级的 `data/` 目录，不回退系统数据目录，不提供修改数据路径的设置项。

## 技术栈

| 层 | 选择 |
|---|---|
| 桌面框架 | Tauri 2 |
| 前端 | Vue 3 + TypeScript + Vite + Tailwind CSS v4 |
| 数据库 | SQLite，经 `tauri-plugin-sql` 访问，是唯一数据真源 |
| 职责划分 | 业务规则与 SQL 在 TypeScript；窗口、托盘、快捷键等系统行为与迁移声明在 Rust |
| 迁移 | 复用 SQLx 迁移机制与 `_sqlx_migrations` 账本 |

## 环境要求

- Node.js 24 及以上、npm
- Rust stable 工具链
- Windows：MSVC 生成工具（Visual Studio Build Tools）与 WebView2 运行时
- macOS：Xcode 命令行工具与系统 WebView

## 构建与运行

```bash
npm install

# 开发模式（Vite + Tauri 热加载）
npm run desktop:dev

# 类型检查
npm run typecheck

# 便携构建（不产出安装包）
npm run desktop:build -- --no-bundle
```

构建产物位于 `src-tauri/target/release/mirrormind.exe`，首次启动时在该文件同级创建 `data/mirrormind.sqlite3`。开发模式的数据固定落在仓库根目录的 `data/` 下。

## 快捷键与交互

| 操作 | 行为 |
|---|---|
| `Ctrl + Shift + Space`（Windows）、`Cmd + Shift + Space`（macOS） | 显示或收起快速记录；收起会先保存草稿 |
| `Ctrl / Cmd + Enter` | 提交记录并收起窗口 |
| `Esc` | 保留草稿并收起窗口 |

快捷键由系统热键接口注册，被占用时无法得知是哪个程序占用；主窗口提供候选键与本次运行内生效的换键、暂停入口。

## 文档

- [`docs/handoff.md`](docs/handoff.md)：交接文档，记录已确认决策、实现现状与事实边界。
- [`docs/design/tech-plan.md`](docs/design/tech-plan.md)：技术方案，含需求边界、架构、数据模型与阶段规划。

## 许可证

个人自用项目，保留所有权利。未附许可证文件。
