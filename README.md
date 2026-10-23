# memTools

memTools 是一个基于 Tauri 2 的桌面快捷记录工具：用全局热键唤起无边框小窗，把想法、事实、决策或日记写入本机 rsrs 记忆库，并提供最近记忆、语义搜索、到期提醒和 3D 记忆图谱。应用通过 MCP over HTTP（JSON-RPC 2.0）连接 rsrs runtime，不调用 CLI，也不会启动、停止或升级 rsrs。

> **运行前提**：本机需已安装 rsrs，且其 runtime 正在 `http://127.0.0.1:15169` 监听。memTools 不管理 rsrs 进程；连接失败时界面显示 `runtime 不可达`。

## 功能

- **全局热键**：默认 `Ctrl+Alt+R` 唤起或隐藏小窗，焦点自动落在输入框；窗口置顶、无边框、透明，关闭按钮和 `Esc` 仅隐藏到托盘。
- **系统托盘**：左键显示/隐藏面板；右键菜单可显示面板、开关划词悬浮、退出应用。
- **手动记录**：选择主区或日记，从运行时拉取的 23 个分类树根中选择挂点，再选择 type（context / decision / preference / task / emotion / time / skill）。
- **判重流程**：命中疑似重复时提供“改 / 并 / 挂 / 存新条”四种处理，破坏性操作前自动备份。
- **自动化记录**：基于本地规则判档、两轮 recall 判重、关键词判树，按“改 > 并 > 挂 > 新建”的顺序执行；可选调用兼容 OpenAI 的模型端点辅助判档和判树，失败时明确告警并回退本地规则。
- **日记模式**：以 `importance=trivial` 直接落库，跳过判重和挂树。
- **三段模板**：填写【前因】【行为】【后果】自动拼装正文；自由文本会按段落结构补标，仅补标记、不改写内容。
- **标题与任务日期**：自动生成标题；task 类型要求【触发】行，缺失时询问而不猜测日期。
- **最近记忆**：按更新时间倒序浏览，支持分类子树过滤和条数调整。
- **搜索**：内容语义搜索走 rsrs recall；输入 8 位 id 前缀或完整 UUID 时走精确查询，并显示祖先路径。
- **提醒**：扫描含【触发】日期的非日记条目，分为已逾期、今天到期、即将到期和之后；支持忽略与标记完成。
- **3D 记忆图谱**：只读读取 rsrs 的 `onememory.db` 与 `core-index`，构建节点和边；支持聚焦层级、节点上限、大脑容器显示、实时增量刷新（900ms）。
- **浏览器扩展**：MV3 扩展把网页选中文字通过本机端点保存为日记或自动记录，正文附带来源标题和 URL。
- **划词悬浮**（仅 Windows）：拖选或双击选词后弹出小图标，可存为日记或带入主窗编辑。
- **同步状态**：底栏显示离线、待同步数量和冲突数量。
- **自动更新**：从 GitHub Releases 读取清单，下载并校验 SHA-256 后在 Windows 上自替换重启。

## 平台支持

| 能力 | Windows | macOS / Linux |
| --- | --- | --- |
| 记录、判重、自动化、最近记忆、搜索、提醒、图谱、托盘、热键 | 支持 | 支持 |
| 全局划词悬浮 | 支持 | 不支持（空实现，托盘项置灰） |
| 自动更新检查 | 支持 | 支持 |
| 自动更新下载与安装 | 支持 | 不支持（可检查，需手动下载安装包） |

## 前置条件

1. **rsrs 已安装并运行**：runtime 默认监听 `127.0.0.1:15169`。memTools 只作为客户端连接，不负责启动或停止服务。
2. **语义索引就绪**：`rsrs status --json` 中的 `retrieval_index.state` 为 `ready` 时，语义搜索、自动化判重和图谱向量才可用。未就绪时界面显示琥珀色“索引未就绪”徽标。
3. **多账号**：memTools 连接当前在 15169 端口监听的账号 runtime。需要切换账号时，修改 `[rsrs] base_url` 指向对应端口。
4. **鉴权（可选）**：若 rsrs runtime 要求令牌，在 `[rsrs] auth_token` 填写，或设置环境变量 `RSRS_TOKEN` / `RSRS_AUTH_TOKEN`（环境变量优先）。

## 安装与运行

从 GitHub Releases 下载：

- `memtools-<版本>.exe`：免安装独立可执行文件；
- `.msi` / `.exe`（NSIS）：Windows 安装包。

首次启动时，如果可执行文件旁和用户配置目录都没有 `config.toml`，应用会把内置模板写入用户配置目录。之后可编辑该文件调整热键、端口、自动化等行为。

发布包未做商业代码签名：Windows 可能提示“未知发布者”，macOS 需在“系统设置 → 隐私与安全性”中允许打开。macOS 构建为 Apple Silicon（arm64）版本。

## 快速上手

1. 确认 rsrs runtime 已启动，再运行 memTools。
2. 按 `Ctrl+Alt+R` 唤起小窗。
3. 输入内容，或切换到“三段模板”分别填写【前因】【行为】【后果】。
4. 选择“主区”或“日记”；主区再选择分类树和可选 type。
5. 按 `Ctrl+Enter` 提交。主区提交成功后会显示动作、条目 id 和挂载路径。
6. 需要自动判档、判重和判树时，打开底栏“自动化”开关；可填写项目名或关键词提升判重准确度。

## 配置

### 文件查找顺序

| 顺序 | 位置 |
| --- | --- |
| 1 | 环境变量 `MEMTOOLS_CONFIG` 指向的文件（绝对路径） |
| 2 | 可执行文件同目录的 `config.toml`（便携模式） |
| 3 | Windows：`%APPDATA%\com.memtools.desktop\config.toml`；macOS：`~/Library/Application Support/com.memtools.desktop/config.toml`；Linux：`$XDG_CONFIG_HOME/com.memtools.desktop/config.toml`（未设置时用 `~/.config/...`） |
| 4 | 编译进程序的内置默认值 |

debug 构建会在第 3 项之前额外读取源码目录的 `src-tauri/config.toml`，便于开发时直接改模板；release 构建不参与。

### 热键与窗口

| 键 | 默认值 | 说明 |
| --- | --- | --- |
| `hotkey.register` | `Ctrl+Alt+R` | 全局热键，语法由 tauri-plugin-global-shortcut 解析 |
| `window.width` | `480` | 窗口宽度（逻辑像素） |
| `window.height` | `560` | 窗口高度（逻辑像素） |
| `window.always_on_top` | `true` | 窗口置顶 |
| `window.decorations` | `false` | 无系统边框，由前端自绘 |
| `window.transparent` | `true` | 透明窗口 |
| `window.remember_position` | `true` | 记忆上次窗口位置 |
| `window.submit_close` | `true` | 提交成功后短暂回显并自动隐藏 |

### rsrs 连接

| 键 | 默认值 | 说明 |
| --- | --- | --- |
| `rsrs.base_url` | `http://127.0.0.1:15169` | runtime 基础地址 |
| `rsrs.mcp_path` | `mcp` | MCP 端点路径，请求 `POST {base_url}/{mcp_path}` |
| `rsrs.auth_token` | 空 | 可选 Bearer 令牌；环境变量优先 |
| `rsrs.timeout_secs` | `5` | status / taxonomy 快速探测超时（秒） |
| `rsrs.op_timeout_secs` | `30` | remember / recall / update / attach / show / tree 超时（秒） |

### rsrs.tools

服务端工具名映射，通常无需修改。

| 键 | 默认值 | 说明 |
| --- | --- | --- |
| `rsrs.tools.status` | `memory_status` | 查询运行状态与同步信息 |
| `rsrs.tools.taxonomy` | `memory_taxonomy` | 获取分类树根 |
| `rsrs.tools.remember` | `memory_remember` | 新建记忆 |
| `rsrs.tools.recall` | `memory_recall` | 语义召回 |
| `rsrs.tools.tree` | `memory_tree` | 查询子树 |
| `rsrs.tools.show` | `memory_show` | 读取单条全文与祖先路径 |
| `rsrs.tools.update` | `memory_update` | 修改已有条目 |
| `rsrs.tools.attach` | `memory_attach` | 调整父子关系 |
| `rsrs.tools.list` | `memory_list` | 列出最近条目 |
| `rsrs.tools.query_log_mark` | `memory_query_log_mark` | 标记召回候选的采纳情况 |

### 自动化

| 键 | 默认值 | 说明 |
| --- | --- | --- |
| `automation.enabled` | `false` | 启动时自动化开关的初始状态 |
| `automation.mode` | `local-fallback` | `local-fallback`、`high-quality` 或 `llm`；非 `local-fallback` 且模型启用时调用模型 |
| `automation.recall_rounds` | `2` | 判重召回轮数 |
| `automation.recall_limit` | `3` | 每轮候选上限 |
| `automation.dup_threshold` | `0.85` | 相似度达到该值时视为疑似重复，需用户确认“改”或“并” |
| `automation.attach_threshold` | `0.70` | 相似度达到该值时视为同题，自动挂到候选之下 |

### 模型判档与判树

| 键 | 默认值 | 说明 |
| --- | --- | --- |
| `automation.llm.enabled` | `false` | 是否允许调用模型 |
| `automation.llm.endpoint` | 空 | 任意兼容 OpenAI 的 `/v1/chat/completions` 地址；留空不调用 |
| `automation.llm.model` | 空 | 模型名称 |
| `automation.llm.api_key` | 空 | 留空则不发 `Authorization` 头 |
| `automation.llm.timeout_secs` | `60` | 模型请求超时（秒） |

调用失败或未配置时，自动回退本地判档和判树，并在结果中显示告警，不会静默降级。

### 提醒

| 键 | 默认值 | 说明 |
| --- | --- | --- |
| `reminder.enabled` | `true` | 启用界面侧定时扫描 |
| `reminder.interval_minutes` | `30` | 后台扫描间隔（分钟） |
| `reminder.soon_days` | `7` | 未来多少天内视为“即将到期” |

### 浏览器扩展端点

| 键 | 默认值 | 说明 |
| --- | --- | --- |
| `extension.enabled` | `true` | 是否启动本机 HTTP 端点 |
| `extension.port` | `17650` | 仅绑定 `127.0.0.1` |
| `extension.token` | 空 | 固定令牌；留空则首次启动自动生成到 `{app_config_dir}/extension_token.txt` |

### 自动更新

| 键 | 默认值 | 说明 |
| --- | --- | --- |
| `update.enabled` | `true` | 是否启用更新检查 |
| `update.endpoint` | `https://github.com/risense-ai/respire-memtool/releases/latest/download/update.json` | 更新清单地址，可改为任意 HTTPS 地址或本地服务 |
| `update.check_on_start` | `true` | 启动时静默检查一次 |

### 划词悬浮

| 键 | 默认值 | 说明 |
| --- | --- | --- |
| `selection.enabled` | `true` | 启动时是否开启；托盘菜单可随时切换 |
| `selection.min_chars` | `2` | 少于该字数不弹图标 |
| `selection.max_chars` | `4000` | 超过该字数忽略 |
| `selection.drag_threshold_px` | `8` | 拖选判定的位移阈值（物理像素） |
| `selection.capture_timeout_ms` | `600` | 模拟复制后等待剪贴板更新的上限 |
| `selection.double_click` | `true` | 双击选词也弹图标 |
| `selection.offset_px` | `14` | 图标相对光标的偏移 |

## 索引未就绪怎么办

语义索引未完成构建时，底栏出现琥珀色“索引未就绪”徽标，点击可查看说明。此时：

- 自动化判重、语义搜索和图谱向量可能失败；
- 手动记录、最近记忆、id 精确查询仍然可用。

处理方法：在 rsrs 侧完成检索模型初始化并等待索引构建，使用 `rsrs status --json` 查看 `retrieval_index.state`；变为 `ready` 后回到 memTools 重试。

## 浏览器扩展

1. 启动 memTools，确认 `[extension] enabled = true`。
2. 点击底栏“扩展” →“安装 / 配对 Chrome 扩展”，准备本机私有扩展并打开 Chrome 管理页和目录。
3. 在 Chrome 开启开发者模式，选择“加载已解压的扩展程序”，指向已打开的 `chrome-extension` 目录；已加载此目录时点击重新加载。
4. 配对会自动导入，无需复制令牌；可在扩展选项点击“测试连接”。
5. 在网页中选中文字，右键选择保存为日记或自动记录。

Chrome 的安装确认步骤仍需用户完成。备用手动接入仍可加载源码 `extension/` 并填写端口/令牌。
端点只绑定回环地址，并要求令牌。`extension_token.txt` 和用户配置目录中的私有 `chrome-extension/pairing.json` 含本机令牌，请勿提交或分享；公开源码里的令牌为空。

扩展配对回归可运行 `node --test devtools/extension-settings.test.mjs`，Rust 资源导出测试包含在 `npm test`。

## 自动更新与发布

客户端读取 `update.json` 后比较版本，下载新可执行文件并校验 SHA-256，然后由新进程在旧进程退出后完成替换并重启。安装仅支持 Windows。

更新请求优先使用环境代理，未配置时读取 Windows 当前用户的静态系统代理；回环地址始终直连，并支持代理绕过列表。仅配置 PAC 而没有静态代理时不解析 PAC。MCP 客户端兼容 JSON 与 SSE，并独立于更新代理保持直连。

清单字段：`version`、`notes`、`pub_date`、`url`、`sha256`、`size`、`mandatory`。

发布流程：

1. 同时修改 `package.json` 和 `src-tauri/tauri.conf.json` 的版本号。
2. 提交改动。
3. 创建并推送标签：`git tag vX.Y.Z`，`git push origin vX.Y.Z`。
4. GitHub Actions 在 `windows-latest`、`macos-latest`、`ubuntu-22.04` 上分别构建，创建 Release，上传 Windows 独立 exe / MSI / NSIS、macOS dmg、Linux deb / AppImage、`update.json` 和 `SHA256SUMS`。

标签必须与 `package.json` 版本一致（`v` 前缀），否则工作流会失败。也可通过 `workflow_dispatch` 手动触发。

本地验证更新流程：

```bash
npm run update:publish -- "更新说明"
npm run update:serve
```

前者把构建产物和清单写入本地更新目录，后者在 `127.0.0.1:17651` 提供清单；可用 `UPDATE_RATE` 限制下载速度以观察进度。

## 数据与隐私

- memTools 自身不收集或上传数据，不发送遥测；记忆存储在 rsrs 中，其同步行为由 rsrs 配置决定。
- 记忆读写通过 rsrs runtime 完成；图谱以只读方式打开 `onememory.db` 和 `core-index`。
- 浏览器扩展端点仅绑定 `127.0.0.1`，无令牌无法写入。
- 若启用 `[automation.llm]`，待判定的正文会发送到所配置的模型端点；是否为本机服务取决于该地址。
- 更新包会校验 SHA-256，但清单本身未做代码签名。对来源有疑虑时，请自行核对 Release 中公布的哈希。

## 从源码构建

需要 Node.js 20+、Rust stable，以及对应平台的 Tauri 构建依赖（Windows 需 WebView2，macOS 需 Xcode Command Line Tools，Linux 需 WebKitGTK 等）。

```bash
npm ci
npm run tauri dev      # 开发模式
npm test               # Rust 单元测试（cargo test --lib）
npm run tauri build    # 安装包与独立可执行文件
```

只生成免安装 exe：

```bash
npm run tauri build -- --no-bundle
```

产物位于 `src-tauri/target/release/memtools.exe`。不要直接用 `cargo build --release`：缺少 Tauri 的 `custom-protocol` 特性时，得到的程序仍会尝试加载开发服务器地址。

## 已知限制

- 3D 图谱依赖 rsrs 内部的 SQLite 表和 `core-index` 布局，目前以 rsrs 1.0.x 验证；schema 变更可能导致图谱不可用。
- runtime 不可达时不展示图谱，也不会猜测其他账号的数据库位置。
- 全局划词悬浮仅支持 Windows；需要 `Ctrl+Shift+C` 复制的终端类应用可能取不到选中文字。
- 自动更新的下载与安装仅支持 Windows，其他平台需手动下载。
- 没有账号切换器；切换账号需要调整 `rsrs.base_url` 或让目标账号占用同一端口。
- 发布包未签名；macOS 构建仅覆盖 arm64。

## 许可证

源代码与文档采用 MIT，Copyright (c) 2026 risense-ai。第三方组件及其许可证见 [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md)。

`src/graph/assets/brain2.container.ply` 不在此许可范围内，仅限本项目使用，详见 [ASSETS.md](ASSETS.md)。
