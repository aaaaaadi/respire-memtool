/* ============================================================
   memTools 前端主逻辑（原生 JS）
   依赖 Tauri 2 注入的全局 API：window.__TAURI__
   ============================================================ */

import { GraphView } from "./graph/graphView.js";

const invoke = () => window.__TAURI__?.core?.invoke;
const listen = () => window.__TAURI__?.event?.listen;

// ---- DOM 引用 ----
const $ = (sel) => document.querySelector(sel);
const inputEl = $("#memory-input");
const inputWrap = $(".input-wrap");
const sectionsWrap = $("#sections-wrap");
const secCause = $("#sec-cause");
const secAction = $("#sec-action");
const secEffect = $("#sec-effect");
const triggerRow = $("#trigger-row");
const triggerInput = $("#trigger-input");
const viewTabs = $("#view-tabs");
const viewWrite = $("#view-write");
const viewBrowse = $("#view-browse");
const viewSearch = $("#view-search");
const viewRemind = $("#view-remind");
const viewGraph = $("#view-graph");
const remindBadge = $("#remind-badge");
const remindStatus = $("#remind-status");
const remindGroups = $("#remind-groups");
const remindRefresh = $("#remind-refresh");
const syncBadge = $("#sync-badge");
const indexBadge = $("#index-badge");
const graphStage = $("#graph-stage");
const graphCanvas = $("#graph-canvas");
const graphStatus = $("#graph-status");
const graphLoading = $("#graph-loading");
const graphLoadingText = $("#graph-loading-text");
const graphEmpty = $("#graph-empty");
const graphDetail = $("#graph-detail");
const graphZoom = $("#graph-zoom");
const graphRotate = $("#graph-rotate");
const graphRebuild = $("#graph-rebuild");
const graphReload = $("#graph-reload");
const graphExpand = $("#graph-expand");
const graphDisplay = $("#graph-display");
const graphMenu = $("#graph-menu");
const graphContainerSwitch = $("#graph-container-switch");
const graphIdleDim = $("#graph-idle-dim");
const graphLevels = $("#graph-levels-input");
const graphMaxNodes = $("#graph-max-nodes");
const browseCount = $("#browse-count");
const browseRefresh = $("#browse-refresh");
const browseStatus = $("#browse-status");
const browseCats = $("#browse-cats");
const browseList = $("#browse-list");
const searchInput = $("#search-input");
const searchBtn = $("#search-btn");
const searchCount = $("#search-count");
const searchStatus = $("#search-status");
const searchList = $("#search-list");
const resultArea = $("#result-area");
const level1 = $("#level-1");
const level2 = $("#level-2");
const level3 = $("#level-3");
const treeGroups = $("#tree-groups");
const typePills = $("#type-pills");
const runtimeDot = $("#runtime-dot");
const autoSwitch = $("#auto-switch");
const extBtn = $("#ext-btn");
const extPanel = $("#ext-panel");
const extPort = $("#ext-port");
const extToken = $("#ext-token");
const extCopy = $("#ext-copy");
const extClose = $("#ext-close");
const updateBtn = $("#update-btn");
const updatePop = $("#update-pop");
const updateTitle = $("#update-title");
const updateBody = $("#update-body");
const updateBar = $("#update-bar");
const updateBarFill = $("#update-bar-fill");
const updateInstall = $("#update-install");
const updateClose = $("#update-close");
const autoState = $("#auto-state");
const hintWrap = $("#hint-wrap");
const hintInput = $("#hint-input");
const charCount = $("#char-count");

// ---- 状态 ----
const TYPE_OPTIONS = [
  "context", "decision", "preference", "task", "emotion", "time", "skill",
];
let taxonomy = [];            // [{id,name,domain}]
let selectedTarget = "main";  // main | journal
let selectedTree = null;      // 树根 name
let selectedType = "context";
let autoMode = false;         // 自动化开关
let inputMode = "free";       // free | sections（三段模板）
let currentView = "write";    // write | browse
let browseCategory = null;    // null = 全部；否则分类名
let browseLoaded = false;
let browseDirty = true;       // 写入成功后标记，下次进入浏览时重取
let browseLoading = false;
let browseReq = 0;            // 请求序号：切分类过快时丢弃过期响应
let searchReq = 0;            // 搜索请求序号
let remindReq = 0;
let appConfig = {
  hotkey: "Ctrl+Alt+R",
  submit_close: true,
  port: 15169,
  selection_supported: false,
  update_check_on_start: true,
};
let runtimeStatus = null;

// ---- 3D 图谱状态 ----
let graphView = null;
let graphLoaded = false;
let graphVisible = false;
let graphLoadingFlag = false;
let graphTimer = null;
let graphCursor = { seq: 0, qid: 0 };
let graphExpanded = false;
let graphAutoRotate = false;
let graphPollFailures = 0;

// ---- 工具 ----
function setResult(kind, title, body) {
  resultArea.className = `result-area ${kind}`;
  resultArea.innerHTML = "";
  const t = document.createElement("div");
  t.className = "result-title";
  t.textContent = title;
  const b = document.createElement("div");
  b.className = "result-body";
  b.textContent = body;
  resultArea.appendChild(t);
  resultArea.appendChild(b);
  resultArea.classList.remove("hidden");
}

function clearResult() {
  resultArea.classList.add("hidden");
  resultArea.className = "result-area hidden";
  resultArea.innerHTML = "";
}

function flashOk() {
  runtimeDot.classList.add("online");
  setTimeout(() => {
    runtimeDot.classList.remove("online");
    runtimeDot.classList.add("offline");
  }, 2000);
}

// ---- 输入模式：自由文本 / 三段模板 ----
function setInputMode(mode) {
  inputMode = mode;
  const free = mode === "free";
  document.querySelectorAll(".mode-btn").forEach((b) =>
    b.classList.toggle("active", b.dataset.mode === mode)
  );
  inputWrap.classList.toggle("hidden", !free);
  sectionsWrap.classList.toggle("hidden", free);
  updateCharCount();
  if (free) inputEl.focus();
  else secAction.focus();
}

function sectionContent() {
  const parts = [];
  const push = (marker, el) => {
    const v = el.value.trim();
    if (v) parts.push(`${marker}${v}`);
  };
  push("【前因】", secCause);
  push("【行为】", secAction);
  push("【后果】", secEffect);
  return parts.join("\n");
}

/// 当前待提交正文（三段模式下拼成 【前因】/【行为】/【后果】）
function getContent() {
  return inputMode === "sections" ? sectionContent() : inputEl.value.trim();
}

function updateCharCount() {
  charCount.textContent = `${getContent().length} 字`;
}

// ---- task 类型：【触发】行显隐（日期不猜，缺失时询问）----
function updateTriggerRow(forceShow) {
  const isTask = !autoMode && selectedTarget === "main" && selectedType === "task";
  const hasTrigger = getContent().includes("【触发】");
  const show = forceShow || (isTask && !hasTrigger);
  triggerRow.classList.toggle("hidden", !show);
  return show;
}


function renderTree() {
  treeGroups.innerHTML = "";
  const human = taxonomy.filter((t) => (t.domain || "") === "human-domain" || t.domain === "human");
  const ai = taxonomy.filter((t) => (t.domain || "") === "AI-domain" || t.domain === "ai" || t.domain === "AI");
  const groups = [
    { title: "人域 · Human", items: human },
    { title: "AI 域 · AI", items: ai },
  ];
  for (const g of groups) {
    if (!g.items.length) continue;
    const box = document.createElement("div");
    box.className = "tree-group";
    const label = document.createElement("div");
    label.className = "tree-group-title";
    label.textContent = g.title;
    box.appendChild(label);
    const chips = document.createElement("div");
    chips.className = "tree-chips";
    for (const item of g.items) {
      const chip = document.createElement("button");
      chip.className = "tree-chip";
      chip.textContent = item.name;
      chip.dataset.name = item.name;
      chip.dataset.id = item.id;
      chip.addEventListener("click", () => selectTree(chip));
      chips.appendChild(chip);
    }
    box.appendChild(chips);
    treeGroups.appendChild(box);
  }
}

function selectTree(chip) {
  treeGroups.querySelectorAll(".tree-chip").forEach((c) => c.classList.remove("active"));
  chip.classList.add("active");
  selectedTree = chip.dataset.name;
}

// ---- 渲染：type ----
function renderTypes() {
  typePills.innerHTML = "";
  for (const t of TYPE_OPTIONS) {
    const pill = document.createElement("button");
    pill.className = "type-pill" + (t === "context" ? " active" : "");
    pill.textContent = t;
    pill.dataset.type = t;
    pill.addEventListener("click", () => {
      typePills.querySelectorAll(".type-pill").forEach((p) => p.classList.remove("active"));
      pill.classList.add("active");
      selectedType = t;
      updateTriggerRow(false);
    });
    typePills.appendChild(pill);
  }
}

// ---- 视图切换：记录 / 最近记忆 ----
function switchView(view) {
  currentView = view;
  viewTabs.querySelectorAll(".view-tab").forEach((b) =>
    b.classList.toggle("active", b.dataset.view === view)
  );
  viewWrite.classList.toggle("hidden", view !== "write");
  viewBrowse.classList.toggle("hidden", view !== "browse");
  viewSearch.classList.toggle("hidden", view !== "search");
  viewRemind.classList.toggle("hidden", view !== "remind");
  viewGraph.classList.toggle("hidden", view !== "graph");
  clearResult();
  if (view === "graph") {
    enterGraph();
  } else {
    leaveGraph();
    if (view === "browse") {
      if (!browseLoaded || browseDirty) loadRecent();
    } else if (view === "search") {
      searchInput.focus();
    } else if (view === "remind") {
      loadReminders();
    } else {
      (inputMode === "free" ? inputEl : secAction).focus();
    }
  }
}

// ---------------------------------------------------------------------------
// 3D 图谱：进入/离开、快照、实时轮询、详情
// ---------------------------------------------------------------------------

async function enterGraph() {
  if (!graphView) {
    graphView = new GraphView(graphCanvas, {
      onNodeClick: showGraphDetail,
      onStatus: (s) => updateGraphStatus(s || {}),
    });
  }
  graphVisible = true;
  graphView.setActive(true);
  if (!graphExpanded) await setGraphExpanded(true);
  if (!graphLoaded) {
    await loadGraphSnapshot();
  } else {
    graphView.resize();
    startGraphPolling();
    // 切回图谱时先补一次增量，避免错过离开期间的变更
    pollGraph();
  }
}

function leaveGraph() {
  graphVisible = false;
  stopGraphPolling();
  hideGraphDetail();
  if (graphView) {
    graphView.setActive(false);
    graphView.clearFocus();
  }
  if (graphExpanded) setGraphExpanded(false);
}

async function setGraphExpanded(expanded) {
  graphExpanded = !!expanded;
  graphExpand.textContent = graphExpanded ? "还原" : "放大";
  try {
    await invoke()("set_graph_window", { expanded: graphExpanded });
  } catch (_) {
    // 非 Tauri 或窗口命令失败时不影响图谱本身
  }
  setTimeout(() => graphView?.resize(), 120);
}

async function loadGraphSnapshot() {
  if (graphLoadingFlag) return;
  graphLoadingFlag = true;
  stopGraphPolling();
  graphLoading.classList.remove("hidden");
  graphLoadingText.textContent = "正在构建 3D 图谱（首次需读取 m3 向量，约几秒）…";
  graphEmpty.classList.add("hidden");
  graphStatus.textContent = "构建中…";
  try {
    const snap = await invoke()("graph_snapshot");
    if (!snap) throw new Error("Rust 命令无返回");
    if (!graphView) {
      graphView = new GraphView(graphCanvas, {
        onNodeClick: showGraphDetail,
        onStatus: (s) => updateGraphStatus(s || {}),
      });
    }
    graphView.setData(snap);
    graphCursor = snap.cursor || { seq: 0, qid: 0 };
    graphLoaded = true;
    graphPollFailures = 0;
    updateGraphStatus({
      nodes: snap.stats?.nodes ?? snap.nodes?.length ?? 0,
      links: snap.stats?.links ?? snap.links?.length ?? 0,
      note: snap.stats?.note,
    });
    if (!snap.nodes || !snap.nodes.length) {
      showGraphEmpty("记忆库还是空的", "先在「记录」里存一条，图谱会实时长出来。");
    }
    startGraphPolling();
  } catch (e) {
    graphLoaded = false;
    showGraphEmpty("无法加载 3D 图谱", String(e?.message || e), true);
    graphStatus.textContent = "加载失败";
  } finally {
    graphLoadingFlag = false;
    graphLoading.classList.add("hidden");
  }
}

function showGraphEmpty(title, body, retry = false) {
  graphEmpty.innerHTML = "";
  const t = document.createElement("div");
  t.className = "graph-empty-title";
  t.textContent = title;
  const b = document.createElement("div");
  b.className = "graph-empty-body";
  b.textContent = body;
  graphEmpty.appendChild(t);
  graphEmpty.appendChild(b);
  if (retry) {
    const btn = document.createElement("button");
    btn.className = "mini-btn primary";
    btn.textContent = "重试";
    btn.addEventListener("click", () => loadGraphSnapshot());
    graphEmpty.appendChild(btn);
  }
  graphEmpty.classList.remove("hidden");
}

function startGraphPolling() {
  stopGraphPolling();
  graphTimer = setInterval(pollGraph, 900);
}

function stopGraphPolling() {
  if (graphTimer) {
    clearInterval(graphTimer);
    graphTimer = null;
  }
}

async function pollGraph() {
  if (!graphVisible || !graphLoaded || graphLoadingFlag || document.hidden) return;
  const seq = Number(graphCursor.seq) || 0;
  const qid = Number(graphCursor.qid) || 0;
  try {
    const res = await invoke()("graph_events_since", { seq, qid });
    if (!graphVisible || !res) return;
    graphPollFailures = 0;
    if (res.reset) {
      await loadGraphSnapshot();
      return;
    }
    if (Array.isArray(res.ops) && res.ops.length) {
      graphView?.applyOps(res.ops);
      const last = res.ops[res.ops.length - 1];
      if (last?.op) updateGraphStatus({ event: last.op });
    }
    graphCursor = res.cursor || { seq, qid };
    if (res.has_more) setTimeout(pollGraph, 0);
  } catch (e) {
    graphPollFailures += 1;
    if (graphPollFailures === 1) {
      updateGraphStatus({ error: String(e?.message || e) });
    }
    if (graphPollFailures >= 5) {
      stopGraphPolling();
      setTimeout(() => {
        if (graphVisible) startGraphPolling();
      }, 10000);
    }
  }
}

function updateGraphStatus(info = {}) {
  const n = info.nodes ?? graphView?.nodes?.length ?? 0;
  const m = info.links ?? graphView?.links?.length ?? 0;
  const parts = [];
  if (info.msg) parts.push(info.msg);
  parts.push(`${n} 节点 / ${m} 边`);
  if (info.event) parts.push(`最近 ${info.event}`);
  if (info.error) parts.push(`同步失败：${info.error}`);
  else parts.push("实时");
  if (info.containerError) parts.push(`容器：${info.containerError}`);
  if (info.levels) parts.push(`${info.levels} 层 · 上限 ${info.maxNodes ?? 20} 节点`);
  if (info.note) parts.push(info.note);
  graphStatus.textContent = parts.join(" · ");
  graphStatus.title = info.note || "";
}

function showGraphDetail(node) {
  if (!node) {
    hideGraphDetail();
    return;
  }
  graphDetail.innerHTML = "";
  const close = document.createElement("button");
  close.className = "graph-detail-close";
  close.textContent = "×";
  close.title = "关闭";
  close.addEventListener("click", (e) => {
    e.stopPropagation();
    hideGraphDetail();
  });

  const title = document.createElement("div");
  title.className = "graph-detail-title";
  title.textContent = node.name || node.id;

  const meta = document.createElement("div");
  meta.className = "graph-detail-meta";
  meta.textContent = `${node.category || "未分类"} · ${node.kind || "context"} · ${
    node.importance || "normal"
  } · 召回 ${node.recall_count || 0}`;

  const body = document.createElement("div");
  body.className = "graph-detail-body";
  body.textContent = node.content_head || "（无正文预览）";

  const actions = document.createElement("div");
  actions.className = "graph-detail-actions";
  const copy = document.createElement("button");
  copy.className = "mini-btn";
  copy.textContent = "复制 id";
  copy.addEventListener("click", async (e) => {
    e.stopPropagation();
    try {
      await navigator.clipboard.writeText(node.id);
      copy.textContent = "已复制";
    } catch (_) {
      copy.textContent = "失败";
    }
    setTimeout(() => (copy.textContent = "复制 id"), 1200);
  });
  const search = document.createElement("button");
  search.className = "mini-btn";
  search.textContent = "按 id 搜索";
  search.addEventListener("click", (e) => {
    e.stopPropagation();
    switchView("search");
    searchInput.value = node.id;
    runSearch();
  });
  actions.append(copy, search);

  graphDetail.append(close, title, meta);
  if (Array.isArray(node.tags) && node.tags.length) {
    const tags = document.createElement("div");
    tags.className = "graph-detail-tags";
    tags.textContent = node.tags.map((t) => `#${t}`).join(" ");
    graphDetail.appendChild(tags);
  }
  graphDetail.append(body, actions);
  graphDetail.classList.remove("hidden");
}

function hideGraphDetail() {
  graphDetail.classList.add("hidden");
  graphDetail.innerHTML = "";
}

async function rebuildGraphLayout() {
  if (graphLoadingFlag) return;
  graphLoadingFlag = true;
  stopGraphPolling();
  graphLoading.classList.remove("hidden");
  graphLoadingText.textContent = "正在重算 PCA 布局（首次可能几秒）…";
  try {
    const snap = await invoke()("graph_rebuild_layout");
    graphView.setData(snap);
    graphCursor = snap.cursor || graphCursor;
    graphLoaded = true;
    updateGraphStatus({
      nodes: snap.stats?.nodes ?? snap.nodes?.length ?? 0,
      links: snap.stats?.links ?? snap.links?.length ?? 0,
      note: snap.stats?.note,
    });
    startGraphPolling();
  } catch (e) {
    updateGraphStatus({ error: String(e?.message || e) });
  } finally {
    graphLoadingFlag = false;
    graphLoading.classList.add("hidden");
  }
}

// ---- 最近记忆：分类筛选 + 条数 ----
function renderBrowseCats() {
  browseCats.innerHTML = "";
  const mk = (label, value) => {
    const b = document.createElement("button");
    b.className = "cat-chip" + (value === browseCategory ? " active" : "");
    b.textContent = label;
    b.dataset.cat = value ?? "";
    b.addEventListener("click", () => {
      browseCategory = value;
      browseCats.querySelectorAll(".cat-chip").forEach((c) =>
        c.classList.toggle("active", (c.dataset.cat || null) === browseCategory)
      );
      loadRecent();
    });
    return b;
  };
  browseCats.appendChild(mk("全部", null));
  for (const item of taxonomy) browseCats.appendChild(mk(item.name, item.name));
}

function fmtTime(iso) {
  if (!iso) return "-";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso.slice(0, 16).replace("T", " ");
  const p = (n) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

async function loadRecent() {
  const req = ++browseReq;
  browseLoading = true;
  browseStatus.className = "browse-status";
  browseStatus.textContent = "读取中…";
  const limit = Math.max(1, Math.min(200, parseInt(browseCount.value, 10) || 20));
  browseCount.value = limit;
  try {
    const res = await invoke()("list_recent", { limit, category: browseCategory });
    if (req !== browseReq) return; // 已被更新的请求取代
    renderRecent(res);
    browseLoaded = true;
    browseDirty = false;
  } catch (e) {
    if (req !== browseReq) return;
    browseList.innerHTML = "";
    browseStatus.className = "browse-status err";
    browseStatus.textContent = `读取失败：${e?.message || e}`;
  } finally {
    if (req === browseReq) browseLoading = false;
  }
}

function renderRecent(res) {
  const items = res?.items || [];
  browseList.innerHTML = "";
  const scanned = res?.scanned ?? items.length;
  browseStatus.textContent = res?.category
    ? `「${res.category}」${items.length} 条 / 扫描最近 ${scanned} 条${res?.truncated ? "（已达上限，可能不全）" : ""}`
    : `全部类别最近 ${items.length} 条`;

  if (!items.length) {
    const empty = document.createElement("div");
    empty.className = "browse-empty";
    empty.textContent = res?.category
      ? `「${res.category}」最近 ${scanned} 条里没有匹配条目。`
      : "还没有记忆。";
    browseList.appendChild(empty);
    return;
  }

  const frag = document.createDocumentFragment();
  for (const it of items) {
    frag.appendChild(buildMemItem(it));
  }
  browseList.appendChild(frag);
}

/// 统一的记忆卡片：标题 + 元信息 + 可展开正文（最近/搜索共用）
function buildMemItem(it) {
  const row = document.createElement("div");
  row.className = "mem-item";

  const head = document.createElement("div");
  head.className = "mem-item-head";
  const dot = document.createElement("span");
  dot.className = `mem-dot ${it.importance || ""}`;
  const title = document.createElement("span");
  title.className = "mem-title";
  title.textContent = it.title || "(无标题)";
  head.appendChild(dot);
  head.appendChild(title);

  const meta = document.createElement("div");
  meta.className = "mem-meta";
  const parts = [];
  if (it.updated_at || it.created_at) {
    parts.push(fmtTime(it.updated_at || it.created_at));
  } else if (typeof it.score === "number") {
    parts.push(`相似度 ${Math.round(it.score * 100)}%`);
  }
  parts.push(it.kind || "context");
  if (it.importance) parts.push(it.importance);
  meta.textContent = parts.join(" · ");

  const content = document.createElement("div");
  content.className = "mem-content";
  content.textContent = it.content || "";

  row.addEventListener("click", () => row.classList.toggle("expanded"));
  row.appendChild(head);
  row.appendChild(meta);
  row.appendChild(content);

  if (it.superseded_by) {
    const note = document.createElement("div");
    note.className = "mem-super";
    note.textContent = `已被 ${it.superseded_by} 取代 · 点击查看新条`;
    note.addEventListener("click", (e) => {
      e.stopPropagation();
      switchView("search");
      searchInput.value = it.superseded_by;
      runSearch();
    });
    row.appendChild(note);
  }
  return row;
}

// ---- 搜索：内容语义（recall）/ id 精确 ----
async function runSearch() {
  const query = searchInput.value.trim();
  if (!query) {
    searchStatus.className = "browse-status";
    searchStatus.textContent = "请输入要搜索的内容，或粘贴 8 位 id。";
    return;
  }
  const limit = Math.max(1, Math.min(100, parseInt(searchCount.value, 10) || 20));
  searchCount.value = limit;
  const req = ++searchReq;
  searchStatus.className = "browse-status";
  searchStatus.title = "";
  searchStatus.textContent = "搜索中…";
  try {
    const res = await invoke()("search_memory", { query, limit });
    if (req !== searchReq) return;
    renderSearch(res);
  } catch (e) {
    if (req !== searchReq) return;
    searchList.innerHTML = "";
    searchStatus.className = "browse-status err";
    searchStatus.textContent = `搜索失败：${e?.message || e}`;
  }
}

function renderSearch(res) {
  const items = res?.items || [];
  searchList.innerHTML = "";
  searchStatus.className = "browse-status";
  searchStatus.title = res?.message || "";
  if (res?.message) {
    searchStatus.textContent = res.message;
  } else if (res?.mode === "recall") {
    searchStatus.textContent = `语义匹配 ${items.length} 条`;
  } else {
    searchStatus.textContent = items.length ? "id 精确命中" : "未找到";
  }

  if (!items.length) {
    const empty = document.createElement("div");
    empty.className = "browse-empty";
    empty.textContent = "没有匹配的记忆。";
    searchList.appendChild(empty);
    return;
  }
  const frag = document.createDocumentFragment();
  for (const it of items) frag.appendChild(buildMemItem(it));
  searchList.appendChild(frag);
}

// ---- 提醒：含【触发】日期的 task 到期核对 ----
const REMIND_GROUPS = [
  { key: "overdue", label: "已逾期" },
  { key: "today", label: "今天到期" },
  { key: "soon", label: "即将到期" },
  { key: "later", label: "之后" },
];

function updateRemindBadge(overdue, today) {
  const n = (overdue || 0) + (today || 0);
  if (n > 0) {
    remindBadge.textContent = String(n);
    remindBadge.classList.remove("hidden");
  } else {
    remindBadge.classList.add("hidden");
  }
}

async function loadReminders() {
  if (!window.__TAURI__?.core) return;
  const req = ++remindReq;
  remindStatus.className = "browse-status";
  remindStatus.textContent = "扫描中…";
  try {
    const res = await invoke()("list_reminders");
    if (req !== remindReq) return;
    renderReminders(res);
    updateRemindBadge(res?.overdue, res?.today);
  } catch (e) {
    if (req !== remindReq) return;
    remindGroups.innerHTML = "";
    remindStatus.className = "browse-status err";
    remindStatus.textContent = `提醒扫描失败：${e?.message || e}`;
  }
}

function renderReminders(res) {
  const items = res?.items || [];
  remindGroups.innerHTML = "";
  remindStatus.className = "browse-status";
  const parts = [
    `逾期 ${res?.overdue ?? 0}`,
    `今天 ${res?.today ?? 0}`,
    `即将 ${res?.soon ?? 0}`,
  ];
  if (res?.truncated) {
    parts.push(`仅扫描最近 ${res?.scanned ?? 0} 条，更早任务可能未纳入`);
  }
  remindStatus.textContent = `核对日期 ${res?.checked_at || "-"} · ${parts.join(" · ")}`;

  if (!items.length) {
    const empty = document.createElement("div");
    empty.className = "browse-empty";
    empty.textContent = "没有带【触发】日期的 task。存 task 时填好日期就会出现在这里。";
    remindGroups.appendChild(empty);
    return;
  }

  const frag = document.createDocumentFragment();
  for (const g of REMIND_GROUPS) {
    const group = items.filter((i) => i.status === g.key);
    if (!group.length) continue;
    const box = document.createElement("section");
    box.className = `remind-group ${g.key}`;
    const head = document.createElement("div");
    head.className = "remind-group-head";
    head.textContent = `${g.label} · ${group.length}`;
    box.appendChild(head);
    for (const it of group) box.appendChild(buildReminderItem(it));
    frag.appendChild(box);
  }
  remindGroups.appendChild(frag);
}

function reminderDaysLabel(it) {
  if (it.days < 0) return `逾期 ${-it.days} 天`;
  if (it.days === 0) return "今天";
  return `${it.days} 天后`;
}

function buildReminderItem(it) {
  const row = document.createElement("div");
  row.className = `remind-item ${it.status || ""}`;

  const head = document.createElement("div");
  head.className = "remind-item-head";
  const due = document.createElement("span");
  due.className = "remind-due";
  due.textContent = it.due || "-";
  const title = document.createElement("span");
  title.className = "remind-title";
  title.textContent = it.title || "(无标题)";
  const days = document.createElement("span");
  days.className = "remind-days";
  days.textContent = reminderDaysLabel(it);
  head.append(due, title, days);

  const trigger = document.createElement("div");
  trigger.className = "remind-trigger";
  trigger.textContent = it.trigger ? `【触发】${it.trigger}` : "";

  const content = document.createElement("div");
  content.className = "mem-content";
  content.textContent = it.content || "";

  const actions = document.createElement("div");
  actions.className = "remind-actions";
  const done = document.createElement("button");
  done.className = "mini-btn primary";
  done.textContent = "完成";
  done.title = "在记忆正文追加【完成】并停止提醒（完成前自动备份原文）";
  done.addEventListener("click", async (e) => {
    e.stopPropagation();
    done.disabled = true;
    done.textContent = "完成中";
    try {
      await invoke()("reminder_complete", { id: it.id });
      loadReminders();
    } catch (err) {
      done.disabled = false;
      done.textContent = "完成失败";
      done.title = String(err?.message || err);
      setTimeout(() => {
        done.textContent = "完成";
        done.title = "在记忆正文追加【完成】并停止提醒（完成前自动备份原文）";
      }, 1800);
    }
  });
  const dismiss = document.createElement("button");
  dismiss.className = "mini-btn";
  dismiss.textContent = "忽略";
  dismiss.title = "不再提示（该条触发日期变更后会重新出现）";
  dismiss.addEventListener("click", async (e) => {
    e.stopPropagation();
    dismiss.disabled = true;
    try {
      await invoke()("reminder_dismiss", { id: it.id, due: it.due });
      loadReminders();
    } catch (err) {
      dismiss.disabled = false;
      dismiss.textContent = "忽略失败";
      dismiss.title = String(err?.message || err);
    }
  });
  const find = document.createElement("button");
  find.className = "mini-btn";
  find.textContent = "查看";
  find.addEventListener("click", (e) => {
    e.stopPropagation();
    switchView("search");
    searchInput.value = it.id;
    runSearch();
  });
  actions.append(done, dismiss, find);

  row.addEventListener("click", () => row.classList.toggle("expanded"));
  row.append(head, trigger, content, actions);
  return row;
}

// ---- 层一切换 ----
function setTarget(target) {
  selectedTarget = target;
  document.querySelectorAll(".seg-btn").forEach((b) =>
    b.classList.toggle("active", b.dataset.target === target)
  );
  const isMain = target === "main";
  const showTree = isMain && !autoMode;
  level2.classList.toggle("hidden", !showTree);
  level3.classList.toggle("hidden", !showTree);
  level1.classList.toggle("hidden", autoMode);
  if (!isMain) {
    clearResult();
  } else if (showTree && !selectedTree && taxonomy.length) {
    // 保持默认树选择
    const first = treeGroups.querySelector(".tree-chip");
    if (first) selectTree(first);
  }
  updateTriggerRow(false);
}

// ---- 自动化开关 ----
function setAuto(on) {
  autoMode = !!on;
  autoSwitch.checked = autoMode;
  const modelJudges =
    !!appConfig.llm_enabled && appConfig.automation_mode !== "local-fallback";
  autoState.textContent = autoMode
    ? modelJudges
      ? "模型判档·判树"
      : "本地判档·判树"
    : "手动";
  hintWrap.classList.toggle("hidden", !autoMode);
  // 自动化开启时隐藏手动层级，交由引擎判档/判树
  level1.classList.toggle("hidden", autoMode);
  level2.classList.toggle("hidden", autoMode || selectedTarget !== "main");
  level3.classList.toggle("hidden", autoMode || selectedTarget !== "main");
  updateTriggerRow(false);
  clearResult();
}

// ---- 提交 ----
async function submit() {
  const content = getContent();
  if (inputMode === "sections" && !secAction.value.trim()) {
    setResult("warn", "【行为】为空", "三段模板中「行为」必填：写出做了什么。");
    return;
  }
  if (!content) {
    setResult("warn", "内容为空", "请先输入要记录的内容。");
    return;
  }
  if (content.length > 8000) {
    setResult("err", "内容过长", `已 ${content.length} 字，超过 8000 字上限；请拆分后分条记录。`);
    return;
  }
  if (!autoMode && selectedTarget === "main" && !selectedTree) {
    setResult("warn", "未选择分类", "主区需要选择一棵分类树，或切换为日记。");
    return;
  }
  clearResult();
  const trigger = triggerInput.value.trim() || null;
  try {
    if (autoMode) {
      const res = await invoke()("automation_submit", {
        content,
        hint: hintInput.value.trim() || null,
      });
      await handleAutomation(res, content);
    } else {
      const res = await invoke()("submit_memory", {
        target: selectedTarget,
        content,
        tree: selectedTarget === "main" ? selectedTree : null,
        type: selectedTarget === "main" ? selectedType : null,
        trigger,
      });
      if (!res) throw new Error("Rust 命令无返回");
      if (res.ok) {
        resetInputs();
        await showDone(res.action, res.id, res.parent, res.message, null, null, res.title, null);
      } else if (res.needs_trigger) {
        updateTriggerRow(true);
        triggerInput.focus();
        setResult("warn", "需要【触发】", res.message || "请填写确认的日期或可核实前提。");
      } else if (res.needs_confirm && (res.candidates || []).length) {
        renderChoices(res.candidates, content, "判重命中相似记忆，请择一：");
      } else {
        setResult("err", "提交失败", res.message || "未知错误，请检查 rsrs runtime。");
      }
    }
  } catch (e) {
    setResult("err", "提交失败", String(e?.message || e));
  }
}

/// 落库成功后清空输入（三段/自由/触发行一并复位）
function resetInputs() {
  inputEl.value = "";
  secCause.value = "";
  secAction.value = "";
  secEffect.value = "";
  triggerInput.value = "";
  hintInput.value = "";
  updateCharCount();
  updateTriggerRow(false);
  browseDirty = true;
}

// ---- 自动化结果处理 ----
async function handleAutomation(res, content) {
  if (!res) throw new Error("Rust 命令无返回");
  if (res.needs_confirm || res.phase === "plan") {
    const head = res.message || "请择一执行：改 / 并 / 挂 / 存新条。";
    renderChoices(res.candidates || [], content, head);
    return;
  }
  if (res.ok) {
    resetInputs();
      await showDone(res.action, res.id, res.parent, res.message, res.branch, res.importance, res.title, "local");
  } else {
    setResult("err", "自动化失败", res.message || "未知错误。");
  }
}

async function showDone(action, id, parent, message, branch, importance, title, source) {
  let body =
    `action  : ${action ?? "-"}\n` +
    `id      : ${id ?? "-"}\n` +
    `挂点    : ${parent ?? "（日记，并入当日轨迹）"}\n` +
    (title ? `标题    : ${title}\n` : "") +
    (branch ? `分支    : ${branch}${importance ? `（${importance}）` : ""}\n` : "") +
    (source ? `判档    : ${source === "model" ? "模型" : "本地"}\n` : "") +
    (message ? `\n${message}` : "");
  body += await donePathLines(id, parent, branch);
  setResult("ok", "已落库 ✓", body);
  flashOk();
  if (appConfig.submit_close) {
    setTimeout(() => hideWindow(), 1200);
  }
}

/// 结果里的挂点可视化：祖先路径 + attach 时的目标树局部
async function donePathLines(id, parent, branch) {
  if (!window.__TAURI__?.core) return "";
  let out = "";
  if (id) {
    try {
      const p = await invoke()("memory_path", { id });
      if (p?.path?.length > 1 && p.path_text) {
        out += `\n路径    : ${p.path_text}`;
      }
    } catch (_) {}
  }
  if (branch === "attach" && parent) {
    try {
      const nodes = await invoke()("memory_outline", { id: parent, depth: 1 });
      const list = (nodes || []).slice(0, 8);
      if (list.length > 1) {
        out +=
          "\n目标树  :\n" +
          list.map((n) => `${"  ".repeat(Math.max(0, n.depth))}· ${n.title}`).join("\n");
      }
    } catch (_) {}
  }
  return out;
}

// ---- 候选择一 UI（改 / 并 / 挂 / 存新条）----
function renderChoices(candidates, content, title) {
  resultArea.className = "result-area warn";
  resultArea.innerHTML = "";
  const t = document.createElement("div");
  t.className = "result-title";
  t.textContent = title;
  resultArea.appendChild(t);

  if (candidates.length > 0) {
    const list = document.createElement("div");
    list.className = "choice-list";
    candidates.forEach((c, i) => {
      const row = document.createElement("label");
      row.className = "choice-item";
      const cb = document.createElement("input");
      cb.type = "checkbox";
      cb.dataset.id = c.id;
      if (i === 0) cb.checked = true;
      const ttl = document.createElement("span");
      ttl.className = "choice-title";
      ttl.textContent = c.title || c.id;
      const sc = document.createElement("span");
      sc.className = "choice-score";
      sc.textContent = c.score ? `${Math.round(c.score * 100)}%` : "";
      row.appendChild(cb);
      row.appendChild(ttl);
      row.appendChild(sc);
      list.appendChild(row);
    });
    resultArea.appendChild(list);
  }

  const actions = document.createElement("div");
  actions.className = "choice-actions";
  const mk = (act, label) => {
    const b = document.createElement("button");
    b.className = "choice-btn";
    b.dataset.act = act;
    b.textContent = label;
    b.addEventListener("click", () => doResolve(act, content));
    return b;
  };
  if (candidates.length > 0) {
    actions.appendChild(mk("update", "改"));
    actions.appendChild(mk("merge", "并"));
    actions.appendChild(mk("attach", "挂"));
  }

  const sel = document.createElement("select");
  sel.id = "choice-tree";
  sel.className = "choice-tree";
  for (const item of taxonomy) {
    const o = document.createElement("option");
    o.value = item.name;
    o.textContent = item.name;
    if (item.name === selectedTree) o.selected = true;
    sel.appendChild(o);
  }
  const createBtn = document.createElement("button");
  createBtn.className = "choice-btn primary";
  createBtn.textContent = "存新条";
  createBtn.addEventListener("click", () => doResolve("create", content));
  actions.appendChild(sel);
  actions.appendChild(createBtn);

  resultArea.appendChild(actions);
  resultArea.classList.remove("hidden");
}

async function doResolve(choice, content) {
  const checked = Array.from(resultArea.querySelectorAll(".choice-item input:checked")).map(
    (c) => c.dataset.id
  );
  if (choice !== "create" && checked.length === 0) {
    setResult("warn", "未选择候选", "请先勾选一条候选记忆。");
    return;
  }
  const treeSel = resultArea.querySelector("#choice-tree");
  const payload = {
    content,
    choice,
    candidate: choice === "merge" ? checked.join(",") : checked[0] || null,
    tree: choice === "create" ? (treeSel ? treeSel.value : selectedTree) : null,
  };
  try {
    const res = await invoke()("automation_resolve", payload);
    if (!res) throw new Error("Rust 命令无返回");
    if (res.ok) {
      resetInputs();
    await showDone(res.action, res.id, res.parent, res.message, res.branch, res.importance, res.title, res.source);
    } else {
      setResult("err", "执行失败", res.message || "未知错误。");
    }
  } catch (e) {
    setResult("err", "执行失败", String(e?.message || e));
  }
}

// ---- 窗口控制 ----
async function hideWindow() {
  try { await invoke()("hide_window"); } catch (_) {}
}
// ✕ 只隐藏到托盘（进程常驻，热键与托盘继续可用）；真正退出走托盘菜单「退出 memTools」。
async function closeApp() {
  await hideWindow();
}

// ---- 窗口拖拽（无边框窗口）----
// 标题栏整条可拖（含品牌文字等子元素），按钮/输入等交互元素除外。
// 依赖能力 core:window:allow-start-dragging；若 window API 不可用则退回原生 data-tauri-drag-region。
function initDrag() {
  const bar = document.getElementById("titlebar");
  if (!bar) return;
  const win = window.__TAURI__?.window?.getCurrentWindow?.();
  if (!win || typeof win.startDragging !== "function") {
    bar.setAttribute("data-tauri-drag-region", "");
    return;
  }
  bar.style.cursor = "move";
  bar.addEventListener("mousedown", (e) => {
    if (e.button !== 0) return;
    if (e.target.closest("button, input, select, textarea, a, [data-no-drag]")) return;
    e.preventDefault();
    win.startDragging().catch((err) => console.warn("[memTools] startDragging 失败:", err));
  });
}

// ---- 浏览器扩展接入面板 ----
function initExtensionPanel() {
  if (!extBtn || !extPanel) return;
  const close = () => extPanel.classList.add("hidden");

  extBtn.addEventListener("click", async (e) => {
    e.stopPropagation();
    if (!extPanel.classList.contains("hidden")) {
      close();
      return;
    }
    try {
      const info = await invoke()("extension_info");
      extPort.textContent = info.enabled ? String(info.port) : "未启用";
      extToken.value = info.token || "";
      extToken.placeholder = info.token ? "" : "未生成：请重启 memTools";
      extPanel.classList.remove("hidden");
    } catch (err) {
      setResult("err", "无法读取扩展信息", String(err?.message || err));
    }
  });

  extClose?.addEventListener("click", close);

  const install = document.getElementById("ext-install");
  install?.addEventListener("click", async () => {
    install.disabled = true;
    try {
      const result = await invoke()("extension_install");
      document.getElementById("ext-install-hint").textContent = result.message + " 目录：" + result.directory + "。此目录含本机配对信息，请勿分享。";
    } catch (err) {
      document.getElementById("ext-install-hint").textContent = String(err?.message || err);
    } finally {
      install.disabled = false;
    }
  });

  extCopy?.addEventListener("click", async () => {
    const text = extToken.value.trim();
    if (!text) {
      extCopy.textContent = "无令牌";
      setTimeout(() => (extCopy.textContent = "复制令牌"), 1200);
      return;
    }
    let ok = false;
    try {
      await navigator.clipboard.writeText(text);
      ok = true;
    } catch (_) {
      extToken.select();
      try {
        ok = document.execCommand("copy");
      } catch (_) {}
    }
    extCopy.textContent = ok ? "已复制" : "复制失败";
    setTimeout(() => (extCopy.textContent = "复制令牌"), 1200);
  });

  document.addEventListener("click", (e) => {
    if (extPanel.classList.contains("hidden")) return;
    if (extPanel.contains(e.target) || extBtn.contains(e.target)) return;
    close();
  });
}

// ---- 自动更新 ----

let updateInfo = null;
let updateBusy = false;

function fmtBytes(n) {
  const units = ["B", "KB", "MB", "GB"];
  let v = Number(n) || 0;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(i ? 1 : 0)} ${units[i]}`;
}

function showUpdatePop() {
  updatePop.classList.remove("hidden");
}

function hideUpdatePop() {
  updatePop.classList.add("hidden");
}

async function checkUpdate(manual = false) {
  if (!window.__TAURI__?.core) return;
  updateBtn.disabled = true;
  updateBtn.textContent = "检查中";
  try {
    const info = await invoke()("update_check");
    updateInfo = info || null;
    if (info?.has_update) {
      updateBtn.textContent = `更新 v${info.latest}`;
      updateBtn.classList.add("has-update");
      updateTitle.textContent = `发现新版本 v${info.latest}`;
      updateBody.textContent = `${info.notes || "无更新说明"}\n当前 v${info.current} · 安装包 ${fmtBytes(info.size)}`;
      updateInstall.classList.remove("hidden");
      updateBar.classList.add("hidden");
      updateBarFill.style.width = "0%";
      if (manual) showUpdatePop();
    } else {
      updateBtn.textContent = "更新";
      updateBtn.classList.remove("has-update");
      if (manual) {
        updateTitle.textContent = `已是最新 v${info?.current || "?"}`;
        updateBody.textContent = `当前版本 v${info?.current || "?"}`;
        updateInstall.classList.add("hidden");
        showUpdatePop();
      }
    }
  } catch (e) {
    updateInfo = null;
    updateBtn.textContent = "更新";
    updateBtn.classList.remove("has-update");
    if (manual) {
      updateTitle.textContent = "检查更新失败";
      updateBody.textContent = String(e?.message || e);
      updateInstall.classList.add("hidden");
      showUpdatePop();
    }
  } finally {
    updateBtn.disabled = false;
  }
}

async function installUpdate() {
  if (updateBusy) return;
  updateBusy = true;
  updateInstall.disabled = true;
  updateBar.classList.remove("hidden");
  updateBarFill.style.width = "0%";
  updateTitle.textContent = "正在更新";
  updateBody.textContent = "准备下载…";
  try {
    await invoke()("update_install");
    updateBody.textContent = "正在重启…";
  } catch (e) {
    updateTitle.textContent = "更新失败";
    updateBody.textContent = String(e?.message || e);
    updateInstall.disabled = false;
    updateBusy = false;
  }
}

// ---- 初始化 ----
async function init() {
  if (window.__memtoolsInit) return;
  window.__memtoolsInit = true;
  // 兜底守卫：Tauri 2 默认不注入 window.__TAURI__（需 withGlobalTauri: true）。
  // 缺失时明确提示而非静默，UI 骨架仍完整渲染，不阻塞界面。
  if (!window.__TAURI__?.core || !window.__TAURI__?.event) {
    setResult(
      "err",
      "请在 memTools 窗口中使用",
      "当前页面不是 Tauri 运行时（window.__TAURI__ 未注入）。\n" +
        "若你是在浏览器打开 http://localhost:1420，功能不可用。\n" +
        "请改用应用窗口：按全局热键 Ctrl+Alt+R 唤起 memTools 小窗。"
    );
    return;
  }

  // 标题栏拖拽（无边框窗口）
  initDrag();
  // 浏览器扩展接入面板（复制端口/令牌）
  initExtensionPanel();

  // 拉取配置
  try {
    const cfg = await invoke()("get_app_config");
    if (cfg) appConfig = Object.assign(appConfig, cfg);
  } catch (_) {}

  // 拉取分类树
  try {
    taxonomy = (await invoke()("get_taxonomy")) || [];
  } catch (e) {
    taxonomy = [];
    setResult("err", "无法读取分类树", String(e?.message || e));
  }
  renderTree();
  renderTypes();
  renderBrowseCats();
  if (taxonomy.length) {
    const first = treeGroups.querySelector(".tree-chip");
    if (first) selectTree(first);
  }

  // 键盘：Ctrl+Enter 提交（记录视图），Esc 隐藏；浏览视图的条数框回车即刷新
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
      hideWindow();
      return;
    }
    if (e.key === "Enter" && (e.ctrlKey || e.metaKey) && currentView === "write") {
      e.preventDefault();
      submit();
    }
  });

  // 视图切换
  viewTabs.querySelectorAll(".view-tab").forEach((b) =>
    b.addEventListener("click", () => switchView(b.dataset.view))
  );
  browseRefresh.addEventListener("click", loadRecent);
  remindRefresh.addEventListener("click", loadReminders);
  browseCount.addEventListener("change", () => {
    if (currentView === "browse") loadRecent();
  });
  browseCount.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      loadRecent();
    }
  });

  // 搜索
  searchBtn.addEventListener("click", runSearch);
  searchInput.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      runSearch();
    }
  });
  searchCount.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      runSearch();
    }
  });

  // 3D 图谱工具栏
  graphZoom.addEventListener("click", () => graphView?.zoomToFit());
  graphRotate.addEventListener("click", () => {
    graphAutoRotate = !graphAutoRotate;
    graphRotate.classList.toggle("active", graphAutoRotate);
    graphView?.setAutoRotate(graphAutoRotate);
  });
  graphReload.addEventListener("click", () => loadGraphSnapshot());
  graphRebuild.addEventListener("click", () => rebuildGraphLayout());
  graphExpand.addEventListener("click", () => setGraphExpanded(!graphExpanded));

  const closeGraphMenu = () => {
    graphMenu.classList.add("hidden");
    graphDisplay.classList.remove("active");
  };
  graphDisplay.addEventListener("click", (e) => {
    e.stopPropagation();
    const hiddenNow = graphMenu.classList.toggle("hidden");
    graphDisplay.classList.toggle("active", !hiddenNow);
  });
  graphMenu.addEventListener("click", (e) => e.stopPropagation());
  document.addEventListener("click", () => {
    if (!graphMenu.classList.contains("hidden")) closeGraphMenu();
  });
  graphContainerSwitch.addEventListener("change", () => {
    graphView?.setContainerVisible(graphContainerSwitch.checked);
  });
  graphIdleDim.addEventListener("change", () => {
    graphView?.setIdleDim(graphIdleDim.checked);
  });
  const applyGraphLimit = (input, min, max, fallback, apply) => {
    const raw = Number(input.value);
    const v =
      String(input.value).trim() === "" || !Number.isFinite(raw)
        ? fallback
        : Math.max(min, Math.min(max, Math.round(raw)));
    input.value = String(v);
    apply(v);
  };
  graphLevels.addEventListener("change", () => {
    applyGraphLimit(graphLevels, 1, 6, 3, (v) => graphView?.setLevels(v));
  });
  graphMaxNodes.addEventListener("change", () => {
    applyGraphLimit(graphMaxNodes, 1, 500, 20, (v) => graphView?.setMaxNodes(v));
  });

  // 自动更新
  updateBtn.addEventListener("click", () => {
    if (updateInfo?.has_update) showUpdatePop();
    else checkUpdate(true);
  });
  updateClose.addEventListener("click", hideUpdatePop);
  updateInstall.addEventListener("click", installUpdate);
  document.addEventListener("click", (e) => {
    if (updatePop.classList.contains("hidden")) return;
    if (updatePop.contains(e.target) || updateBtn.contains(e.target)) return;
    hideUpdatePop();
  });
  try {
    await listen()("update://progress", (ev) => {
      const { downloaded = 0, total = 0 } = ev.payload || {};
      const pct = total ? Math.min(100, Math.round((downloaded / total) * 100)) : 0;
      updateBarFill.style.width = `${pct}%`;
      updateBody.textContent = `下载中 ${pct}% · ${fmtBytes(downloaded)} / ${fmtBytes(total)}`;
    });
    await listen()("update://restarting", () => {
      updateBody.textContent = "正在重启…";
    });
  } catch (_) {}
  if (appConfig.update_check_on_start !== false) checkUpdate(false);

  // 提醒：后台扫描线程每 interval 分钟广播到期计数
  try {
    await listen()("reminders://due", (ev) => {
      const { overdue = 0, today = 0 } = ev.payload || {};
      updateRemindBadge(overdue, today);
      if (currentView === "remind") loadReminders();
    });
  } catch (_) {}

  // 输入模式切换
  document.querySelectorAll(".mode-btn").forEach((b) =>
    b.addEventListener("click", () => setInputMode(b.dataset.mode))
  );

  // 顶栏按钮
  $("#btn-close").addEventListener("click", closeApp);
  $("#btn-min").addEventListener("click", hideWindow);

  indexBadge.addEventListener("click", showIndexHint);
  indexBadge.addEventListener("keydown", (e) => {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      showIndexHint();
    }
  });

  // 层一
  document.querySelectorAll(".seg-btn").forEach((b) =>
    b.addEventListener("click", () => setTarget(b.dataset.target))
  );

  // 自动化开关
  autoSwitch.addEventListener("change", () => setAuto(autoSwitch.checked));
  setAuto(!!appConfig.automation_enabled);

  // 字数统计（两种模式统一按待提交正文计）
  for (const el of [inputEl, secCause, secAction, secEffect, triggerInput]) {
    el.addEventListener("input", updateCharCount);
  }
  updateCharCount();

  // 全局热键唤起后：焦点落输入框（浏览视图不抢焦点）
  try {
    await listen()("memtools-shown", () => {
      clearResult();
      checkRuntime();
      if (currentView === "write") {
        (inputMode === "free" ? inputEl : secAction).focus();
      } else if (currentView === "search") {
        searchInput.focus();
      } else if (currentView === "remind") {
        loadReminders();
      }
    });
  } catch (_) {}

  // 划词悬浮窗「编辑」→ 预填选中文字，用户再选分类 / 走自动化
  try {
    await listen()("selection-prefill", (ev) => {
      const text = String(ev?.payload ?? "").trim();
      if (!text) return;
      switchView("write");
      setInputMode("free");
      inputEl.value = text;
      updateCharCount();
      updateTriggerRow(false);
      clearResult();
      inputEl.focus();
    });
  } catch (_) {}

  // runtime 连通性检测（前台静默，不阻塞）
  checkRuntime();
  setInterval(() => {
    if (!document.hidden) checkRuntime();
  }, 60000);
}

async function checkRuntime() {
  try {
    const st = await invoke()("runtime_status");
    if (st?.online) {
      runtimeDot.classList.add("online");
      runtimeDot.classList.remove("offline");
      runtimeDot.title = st.message || `rsrs runtime 在线 (127.0.0.1:${appConfig.port})`;
      applySyncBadge(st);
      applyIndexBadge(st);
    } else {
      runtimeDot.classList.add("offline");
      runtimeDot.classList.remove("online");
      runtimeDot.title = st?.message || "rsrs runtime 不可达 — 由宿主启动服务后自动恢复";
      syncBadge.classList.add("hidden");
      hideIndexBadge();
    }
  } catch (_) {
    runtimeDot.classList.add("offline");
    runtimeDot.classList.remove("online");
    runtimeDot.title = "rsrs runtime 不可达";
    syncBadge.classList.add("hidden");
    hideIndexBadge();
  }
}

/// 底栏同步徽标：离线不丢，提示待补传条数（历史冲突只放 tooltip，避免常驻噪音）
function applySyncBadge(st) {
  const pending = Number(st?.pending) || 0;
  const conflicts = Number(st?.conflicts) || 0;
  const bits = [];
  if (st?.offline) bits.push(`离线 · ${pending} 待同步`);
  else if (pending > 0) bits.push(`${pending} 待同步`);
  if (!bits.length) {
    syncBadge.classList.add("hidden");
    return;
  }
  syncBadge.textContent = bits.join(" · ");
  syncBadge.classList.toggle("warn", !!st?.offline);
  const tip = [
    st?.offline
      ? "离线不丢：本地已落库，rsrs 下次 sync 自动补传"
      : "rsrs 后台同步中",
    conflicts > 0 ? `历史冲突 ${conflicts} 条（不影响本地写入）` : "",
    st?.last_sync ? `上次成功同步 ${fmtTime(new Date(Number(st.last_sync) * 1000).toISOString())}` : "",
  ].filter(Boolean);
  syncBadge.title = tip.join(" · ");
  syncBadge.classList.remove("hidden");
}

function applyIndexBadge(st) {
  runtimeStatus = st || null;
  const hint = String(st?.index_hint || "").trim();
  if (!st?.online || st.index_ready !== false || !hint) {
    indexBadge.classList.add("hidden");
    indexBadge.textContent = "";
    indexBadge.title = "";
    return;
  }
  indexBadge.textContent = "索引未就绪";
  indexBadge.title = hint;
  indexBadge.classList.remove("hidden");
}

function hideIndexBadge() {
  runtimeStatus = null;
  indexBadge.classList.add("hidden");
  indexBadge.textContent = "";
  indexBadge.title = "";
}

function showIndexHint() {
  if (!runtimeStatus || runtimeStatus.index_ready !== false) return;
  const hint =
    String(runtimeStatus.index_hint || "").trim() ||
    "语义索引尚未就绪，请稍后重试；期间仍可正常记录，但语义搜索结果可能不全。";
  switchView("write");
  setResult("warn", "索引未就绪", hint);
}

// HMR / 延迟加载时 DOMContentLoaded 可能已经过去，直接 init()
if (document.readyState === "loading") {
  window.addEventListener("DOMContentLoaded", init);
} else {
  init();
}
// 调试入口（控制台/自动化验证用；不改变业务行为）
window.__memtools = {
  submit,
  switchView,
  runSearch,
  loadRecent,
  graph: () => graphView,
  loadGraphSnapshot,
  rebuildGraphLayout,
  pollGraph,
  checkUpdate,
  installUpdate,
};
