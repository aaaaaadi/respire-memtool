# 日记检索与记录界面 Implementation Plan

> For agentic workers: execute task by task; preserve existing dirty changes and do not commit, push, merge or replace the active Runtime without validated binaries.

**Goal:** 跨日期找到日记中的具体记录，限制模型上下文，并在后台提炼有长期价值的记录。
**Architecture:** 保留原日记与同步格式；Runtime 增加派生片段检索，MemTools 复用已有全文筛选并展示命中片段，插件使用受限结果与来源。已有 MemTools 全文筛选不重做。
**Tech Stack:** Rust/SQLite/Core SDK、Tauri/JavaScript、Node Respire hooks。
**Spec:** 本会话用户已确认的四项修复与静默处理；2026-10-26 实测 MemTools memory_list 返回完整 7817 字日记，实际 selectMemories 命中 offset=7764 的港珠澳。

## Global Constraints
- 原始日记不删除、不拆成新的同步记忆；索引属于本机派生数据。
- 现有 skip=0 修复保留。当前 MemTools 只有 rsrs_client.rs 修改和已暂存回归文件。
- 来源包含原记忆 ID、片段 ID、日期及原文位置；显示简短，不自动展开整日日记。
- 关键词查询覆盖完整正文；语义查询只使用本地现有 BGE-M3。
- 返回条数与总长度均有硬上限；旧接口/索引不可用时明确降级，不声称语义成功。
- 提炼后台进行：先查同题、确定真实父节点、保留来源、写后读回、重复任务不重复创建；不擅自覆盖/合并已有正文。
- 不为测试写入真实记忆；可只读验证现有记录。新增持久文件精确暂存。
- 构建依赖缺失不得绕过 SDK 校验或替换未经验证的 Runtime。

## Tasks
- [ ] 1. RSRS：修 diary --contains 的 500 字符限制；增加跨日片段检索与有界邻接读取，维护派生缓存；接入 CLI/MCP 与本地 Core 查询。验证跨日、正文尾部、超长、删除/更新失效、账户隔离、预算。
- [ ] 2. MemTools：复用当前全文筛选；匹配片段显示日期/短来源/高亮，支持打开定位原文及日记筛选。记录区显式显示去向，可搜索选择具体父节点；日记成功回执提供查看入口。
- [ ] 3. MemTools：日记写入后后台提炼长期事实，使用配置模型及既有判重/归属能力；记录来源和幂等状态，错误保留日记，不弹出重复授权。
- [ ] 4. 插件：在现有召回基础上使用片段结果/有界读取；后台提炼保留日记来源；简短来源提示不扩写回合。
- [ ] 5. 运行有针对性的验证、只读真实日记检查、独立 review，打包本机测试版；保留旧程序与输入。

## Review Focus
- 同一日多主题、同一主题多日、仅后半段命中。
- UTF-8 字符位置、重复时间标记、片段过长、相邻读取越界。
- 写入后响应不确定、重复任务、未找到父节点时不得伪报已提炼。
- 老 Runtime 能力缺失、账户切换及敏感缓存目录权限。
- UI 不把整条/整库正文交给对话模型；不可把字数预算宣称精确 token 数。

## Evidence / Rulings
- 2026-10-26：前期把 MemTools 界面全文筛选与 CLI diary --contains 混淆。界面已支持前者，仅补定位与交互，保留已有筛选实现。
- 2026-10-26：Runtime 本地索引 ready；RSRS Core SDK 构建依赖此前缺失，先复核，不承诺未经验证的运行安装。
