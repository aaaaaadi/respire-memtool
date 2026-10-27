/* ============================================================
   memTools 前端主逻辑（原生 JS）
   依赖 Tauri 2 注入的全局 API：window.__TAURI__
   ============================================================ */

import { selectMemories, memoryForest } from "./memory-browser.js";
import { matchingFragments, queryPattern, evidenceLabel, isDiary } from "./diary-fragments.js";

import { GraphView } from "./graph/graphView.js";
import { searchGraphNodes } from "./graph/interaction.js";

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
const viewTargets = $("#view-targets");
const viewGraph = $("#view-graph");
const remindBadge = $("#remind-badge");
const remindStatus = $("#remind-status");
const remindGroups = $("#remind-groups");
const remindRefresh = $("#remind-refresh");
const syncBadge = $("#sync-badge");
const graphStage = $("#graph-stage");
const graphCanvas = $("#graph-canvas");
const graphStatus = $("#graph-status");
const graphLoading = $("#graph-loading");
const graphLoadingText = $("#graph-loading-text");
const graphEmpty = $("#graph-empty");
const graphDetail = $("#graph-detail");
const graphZoom = $("#graph-zoom");
const graphRotate = $("#graph-rotate");
const graphSearch = $("#graph-search");
const graphSearchResults = $("#graph-search-results");
const graphClearSelection = $("#graph-clear-selection");
const graphRebuild = $("#graph-rebuild");
const graphReload = $("#graph-reload");
const graphExpand = $("#graph-expand");
const graphDisplay = $("#graph-display");
const graphMenu = $("#graph-menu");
const graphContainerSwitch = $("#graph-container-switch");
const graphIdleDim = $("#graph-idle-dim");
const graphLevels = $("#graph-levels-input");
const graphMaxNodes = $("#graph-max-nodes");
const browseQuery = $("#browse-query");
const browseViewMode = $("#browse-view-mode");
const searchViewMode = $("#search-view-mode");
const searchKind = $("#search-kind");
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
const processBtn = $("#process-btn");
const processBar = $("#process-bar");
const procSourceOpts = $("#proc-source-opts");
const procSourceHint = $("#proc-source-hint");
const targetsList = $("#targets-list");
const targetsStatus = $("#targets-status");
const targetsAdd = $("#targets-add");
const targetsReset = $("#targets-reset");
const targetsSave = $("#targets-save");
const targetTreeList = $("#target-tree-list");

// ---- 状态 ----
const TYPE_OPTIONS = [
  "context", "decision", "preference", "task", "emotion", "time", "skill",
];
let taxonomy = [];            // [{id,name,domain}]
let selectedTarget = "main";  // main | journal
let selectedTree = null;      // 树根 name
let selectedType = "context";
let selectedParent = null; // 明确选择的真实上级 ID；分类树选择会清空它。
let autoMode = false;         // 自动化开关
let inputMode = "free";       // free | sections（三段模板）
let currentView = "write";    // write | browse
let browseCategory = null;    // null = 全部；否则分类名
let browseLoaded = false;
let browseDirty = true;       // 写入成功后标记，下次进入浏览时重取
let browseLoading = false;
let browseReq = 0;            // 请求序号：切分类过快时丢弃过期响应
let memoryCatalog = null;
let catalogRequest = null;
let catalogRevision = 0;
let browseMode = "tree";
let searchMode = "tree";
let searchKindValue = "quick";
let searchResult = null;
const browseCollapsed = new Set();
const searchCollapsed = new Set();
let searchTimer;
let browseTimer;
let searchReq = 0;            // 搜索请求序号
let remindReq = 0;
let appConfig = { hotkey: "Ctrl+Alt+R", submit_close: true, port: 15169 };

// ---- 快捷目标（P9）----
const MAX_TARGETS = 8;
const TARGET_KINDS = [
  { value: "journal", label: "日记" },
  { value: "tree", label: "挂到节点" },
  { value: "auto", label: "自动判档" },
];
const TARGET_TYPES = [
  "context",
  "decision",
  "preference",
  "task",
  "emotion",
  "time",
  "skill",
];
const TARGET_IMPORTANCES = [
  { value: "important", label: "重要" },
  { value: "trivial", label: "一般" },
];
let targetsDirty = false;
let targetTaxonomy = [];

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
  selectedParent = null;
  updateDestination();
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
  viewTargets.classList.toggle("hidden", view !== "targets");
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
    } else if (view === "targets") {
      loadTargets();
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

/** 搜索已加载图谱；候选使用文本节点，避免记忆标题被当成 HTML 执行。 */
function renderGraphSearch() {
  const query = graphSearch.value.trim();
  graphSearchResults.replaceChildren();
  graphSearchResults.classList.toggle("hidden", !query);
  if (!query) return;
  const matches = searchGraphNodes(graphView?.nodes || [], query);
  const summary = document.createElement("div");
  summary.className = "graph-search-summary";
  summary.setAttribute("role", "status");
  summary.textContent = matches.length ? "匹配 " + matches.length + " 个图谱节点" +
    (matches.length > 30 ? "，先显示 30 个，请补充关键词" : "") : "没有匹配的图谱节点";
  graphSearchResults.appendChild(summary);
  for (const node of matches.slice(0, 30)) {
    const button = document.createElement("button");
    button.className = "graph-search-result";
    const title = document.createElement("strong");
    title.textContent = node.name || node.id;
    const meta = document.createElement("span");
    meta.textContent = [node.category, ...(node.tags || []), node.id].filter(Boolean).join(" · ");
    button.append(title, meta);
    button.addEventListener("click", () => {
      if (!graphView?.selectNode(node.id, { moveCamera: true })) {
        renderGraphSearch();
        return;
      }
      graphSearchResults.classList.add("hidden");
    });
    graphSearchResults.appendChild(button);
  }
}

function updateGraphStatus(info = {}) {
  if (info.autoRotate !== undefined) {
    graphAutoRotate = !!info.autoRotate;
    graphRotate.classList.toggle("active", graphAutoRotate);
  }
  if (info.nodes !== undefined && graphSearch.value.trim() &&
      !graphSearchResults.classList.contains("hidden")) renderGraphSearch();
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
    graphView?.selectNode(null);
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

// ---- 完整快照与即时筛选：切分类、关键字和展示方式不反复请求 Runtime ----
async function ensureMemoryCatalog(force = false) {
  if (force) { browseDirty = true; catalogRevision++; }
  if (catalogRequest) return catalogRequest;
  if (memoryCatalog && !browseDirty) return memoryCatalog;
  catalogRequest = (async () => {
    while (true) {
      const revision = catalogRevision;
      const response = await invoke()("list_recent", { limit: 0, category: null });
      if (response?.truncated) throw new Error("记忆尚未完整读取，请刷新重试。");
      // 读取期间写入或刷新时，旧响应不能清除新的失效标记。
      if (revision !== catalogRevision) continue;
      memoryCatalog = response?.items || [];
      browseDirty = false;
      renderBrowseCats();
      return memoryCatalog;
    }
  })();
  try { return await catalogRequest; }
  finally { catalogRequest = null; }
}

function renderBrowseCats() {
  browseCats.innerHTML = "";
  const make = (label, value) => {
    const button = document.createElement("button");
    button.className = "cat-chip" + (value === browseCategory ? " active" : "");
    button.dataset.cat = value || "";
    button.setAttribute("aria-pressed", String(value === browseCategory));
    const caption = document.createElement("span"); caption.textContent = label;
    button.appendChild(caption);
    if (memoryCatalog) {
      const count = value ? selectMemories(memoryCatalog, { category: value }).length : memoryCatalog.length;
      const badge = document.createElement("small"); badge.textContent = String(count); button.appendChild(badge);
      if (!count) { button.classList.add("empty-category"); button.title = "当前分类暂无记忆"; }
    }
    button.addEventListener("click", () => {
      browseCategory = value;
      browseCats.querySelectorAll(".cat-chip").forEach(chip => {
        const selected = (chip.dataset.cat || null) === browseCategory;
        chip.classList.toggle("active", selected); chip.setAttribute("aria-pressed", String(selected));
      });
      browseCollapsed.clear();
      loadRecent();
    });
    return button;
  };
  browseCats.appendChild(make("全部", null));
  for (const item of taxonomy) browseCats.appendChild(make(item.name, item.name));
}

function fmtTime(iso) {
  if (!iso) return "-";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso.slice(0, 16).replace("T", " ");
  const p = (n) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

async function loadRecent(force = false) {
  const request = ++browseReq;
  browseLoading = true;
  browseStatus.className = "browse-status memory-status";
  browseStatus.textContent = memoryCatalog && !force && !browseDirty ? "筛选中…" : "正在读取全部有效记忆…";
  try {
    await ensureMemoryCatalog(force === true);
    if (request !== browseReq) return;
    renderRecent();
    browseLoaded = true;
  } catch (error) {
    if (request !== browseReq) return;
    browseStatus.className = "browse-status memory-status err";
    browseStatus.textContent = "读取失败：" + String(error?.message || error);
  } finally { if (request === browseReq) browseLoading = false; }
}

function renderRecent() {
  const query = browseQuery.value.trim();
  const candidates = selectMemories(memoryCatalog || [], { category: browseCategory, query }).filter(entry => {
    const kind = $("#browse-source").value;
    return kind === "all" || (kind === "diary" ? isDiary(entry) : !isDiary(entry));
  });
  const limit = Number(browseCount.value) || 0;
  const matches = limit > 0 ? candidates.slice(0, limit) : candidates;
  browseStatus.className = "browse-status memory-status";
  browseStatus.textContent = (browseCategory ? "「" + browseCategory + "」 · " : "") + (query ? "匹配 " : "显示 ") + matches.length + " 条 / 当前范围 " + candidates.length + " 条 · 全库 " + (memoryCatalog?.length || 0) + " 条";
  renderMemoryCollection(browseList, matches, browseMode, browseCollapsed, Boolean(query), browseCategory ? "当前分类暂无匹配记忆。" : "没有匹配的记忆。", query);
}

// 列表与树形共用同一匹配集合；树中补足的祖先只作为路径展示。
function renderMemoryCollection(container, matches, mode, collapsed, searching, emptyMessage, query = "") {
  container.innerHTML = "";
  if (!matches.length) {
    const empty = document.createElement("div"); empty.className = "browse-empty"; empty.textContent = emptyMessage;
    container.appendChild(empty); return;
  }
  if (mode === "list") {
    for (const entry of matches) container.appendChild(buildMemItem(entry, query));
    return;
  }
  const roots = memoryForest(memoryCatalog || [], matches);
  const renderNodes = (nodes, depth) => {
    const list = document.createElement("ul"); list.className = "memory-tree";
    for (const node of nodes) {
      const item = document.createElement("li"); item.className = "memory-tree-node";
      const line = document.createElement("div"); line.className = "memory-tree-line"; line.style.setProperty("--tree-depth", depth);
      const card = buildMemItem(node.entry, node.matched ? query : "");
      if (!node.matched) { card.classList.add("path-context"); card.title = "上级路径，不计入匹配数量"; }
      let children;
      const fold = document.createElement("button"); fold.className = "tree-fold"; fold.type = "button";
      if (node.children.length) {
        children = renderNodes(node.children, depth + 1);
        const open = !collapsed.has(node.entry.id) && (searching || depth === 0);
        const update = expanded => {
          children.hidden = !expanded; fold.textContent = expanded ? "▾" : "▸";
          fold.setAttribute("aria-expanded", String(expanded));
          fold.setAttribute("aria-label", (expanded ? "收起" : "展开") + "：" + (node.entry.title || node.entry.id));
        };
        update(open);
        fold.addEventListener("click", () => {
          const expanded = children.hidden;
          if (expanded) collapsed.delete(node.entry.id); else collapsed.add(node.entry.id);
          update(expanded);
        });
      } else { fold.textContent = "·"; fold.disabled = true; fold.setAttribute("aria-hidden", "true"); }
      line.append(fold, card); item.appendChild(line);
      if (children) item.appendChild(children);
      list.appendChild(item);
    }
    return list;
  };
  container.appendChild(renderNodes(roots, 0));
}

/// 统一的记忆卡片：标题 + 元信息 + 可展开正文（最近/搜索共用）
function buildMemItem(it, query = "") {
  const row = document.createElement("div");
  row.className = "mem-item";
  row.dataset.memoryId = it.id || "";

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
  parts.push(evidenceLabel(it));
  if (it.importance) parts.push(it.importance);
  meta.textContent = parts.join(" · ");

  const content = document.createElement("div");
  content.className = "mem-content";
  content.textContent = it.content || "";

  row.tabIndex = 0;
  row.setAttribute("role", "button");
  row.setAttribute("aria-expanded", "false");
  const toggle = () => row.setAttribute("aria-expanded", String(row.classList.toggle("expanded")));
  row.addEventListener("click", toggle);
  row.addEventListener("keydown", event => {
    if ((event.key === "Enter" || event.key === " ") && event.target === row) { event.preventDefault(); toggle(); }
  });
  row.appendChild(head);
  row.appendChild(meta);
  const fragments = matchingFragments(it.content, query);
  const previews = document.createElement("div");
  previews.className = "mem-fragments";
  const highlight = (element, text) => {
    const pattern = queryPattern(query);
    let cursor = 0;
    for (const match of pattern ? text.matchAll(pattern) : []) {
      element.append(document.createTextNode(text.slice(cursor, match.index)));
      const mark = document.createElement("mark"); mark.textContent = match[0]; element.append(mark);
      cursor = match.index + match[0].length;
    }
    element.append(document.createTextNode(text.slice(cursor)));
  };
  for (const fragment of fragments) {
    const button = document.createElement("button"); button.type = "button"; button.className = "mem-fragment";
    const source = document.createElement("small");
    source.textContent = [fragment.time, (it.id || "").slice(0, 8), "查看原文"].filter(Boolean).join(" · ");
    button.append(source); highlight(button, fragment.text);
    button.addEventListener("click", event => {
      event.stopPropagation();
      row.classList.add("expanded"); row.setAttribute("aria-expanded", "true");
      content.replaceChildren(document.createTextNode((it.content || "").slice(0, fragment.start)));
      const target = document.createElement("span"); highlight(target, fragment.text); content.append(target);
      content.append(document.createTextNode((it.content || "").slice(fragment.end)));
      requestAnimationFrame(() => target.scrollIntoView({block:"center"}));
    });
    previews.append(button);
  }
  row.append(previews, content);

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

// ---- 全库即时关键词搜索 / 保留语义搜索，结果可切换列表与祖先树 ----
async function runSearch() {
  const query = searchInput.value.trim();
  const request = ++searchReq;
  const kind = searchKindValue;
  searchStatus.className = "browse-status memory-status";
  if (!query) { searchResult = null; searchList.innerHTML = ""; searchStatus.textContent = "输入关键词，搜索全部有效记忆。"; return; }
  searchStatus.textContent = searchKindValue === "quick" ? "筛选中…" : "语义搜索中…";
  try {
    let response;
    if (kind === "quick") {
      await ensureMemoryCatalog();
      if (request !== searchReq) return;
      response = { mode: "quick", query, items: selectMemories(memoryCatalog, { query }) };
    } else {
      const limit = Math.max(1, Math.min(100, Number(searchCount.value) || 20));
      response = await invoke()("search_memory", { query, limit });
    }
    if (request !== searchReq) return;
    searchResult = response;
    searchCollapsed.clear();
    renderSearch(response);
    // 语义 / id 查询独立可用；全量快照只补祖先路径，失败时保留命中项。
    if (kind !== "quick" && (!memoryCatalog || browseDirty)) {
      ensureMemoryCatalog().then(() => {
        if (request === searchReq) renderSearch(response);
      }).catch(() => {
        if (request !== searchReq) return;
        response.pathIncomplete = true;
        renderSearch(response);
      });
    }
  } catch (error) {
    if (request !== searchReq) return;
    searchList.innerHTML = "";
    searchStatus.className = "browse-status memory-status err";
    searchStatus.textContent = "搜索失败：" + String(error?.message || error);
  }
}

function renderSearch(response) {
  const items = response?.items || [];
  searchStatus.className = "browse-status memory-status";
  searchStatus.title = response?.message || "";
  searchStatus.textContent = (response?.mode === "quick" ? "全库关键词匹配 " : response?.mode === "recall" ? "语义匹配 " : "精确匹配 ") + items.length + " 条" + (searchMode === "tree" && items.length ? response.pathIncomplete ? " · 上级路径未能读取" : memoryCatalog ? " · 保留上级路径" : " · 正在补全上级路径" : "");
  renderMemoryCollection(searchList, items, searchMode, searchCollapsed, true, "没有匹配的记忆。", response?.mode === "quick" ? response.query : "");
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

// ---------------------------------------------------------------------------
// 快捷目标（P9）：划词面板 / 网页插件共用的有序保存位置列表（第 1 个 = 最优先）
// ---------------------------------------------------------------------------

function setTargetsStatus(text, kind) {
  targetsStatus.className = "browse-status" + (kind ? " " + kind : "");
  targetsStatus.textContent = text || "";
}

function markTargetsDirty() {
  targetsDirty = true;
  setTargetsStatus("有未保存的修改");
}

function fillTargetTreeList() {
  if (!targetTreeList) return;
  targetTreeList.innerHTML = "";
  const names = targetTaxonomy.length ? targetTaxonomy : taxonomy;
  for (const item of names) {
    const name = typeof item === "string" ? item : item && item.name;
    if (!name) continue;
    const opt = document.createElement("option");
    opt.value = name;
    targetTreeList.appendChild(opt);
  }
}

function targetIconBtn(label, title, onClick) {
  const btn = document.createElement("button");
  btn.type = "button";
  btn.className = "mini-btn target-icon";
  btn.textContent = label;
  btn.title = title;
  btn.setAttribute("aria-label", title);
  btn.addEventListener("click", onClick);
  return btn;
}

function syncTargetKind(row) {
  const isTree = row.querySelector(".target-kind").value === "tree";
  row.querySelector(".target-parent").classList.toggle("hidden", !isTree);
  row.querySelector(".target-type").classList.toggle("hidden", !isTree);
  row.querySelector(".target-importance").classList.toggle("hidden", !isTree);
}

function buildTargetRow(target) {
  const row = document.createElement("div");
  row.className = "target-row";
  row.dataset.targetId = target.id || "";

  const head = document.createElement("div");
  head.className = "target-row-head";

  const badge = document.createElement("span");
  badge.className = "target-badge";

  const name = document.createElement("input");
  name.type = "text";
  name.className = "target-name";
  name.maxLength = 24;
  name.value = target.label || "";
  name.placeholder = "目标名称";
  name.spellcheck = false;
  name.addEventListener("input", () => {
    markTargetsDirty();
  });

  const up = targetIconBtn("↑", "上移（更优先）", () => moveTargetRow(row, -1));
  const down = targetIconBtn("↓", "下移", () => moveTargetRow(row, 1));
  const del = targetIconBtn("✕", "删除该目标", () => removeTargetRow(row));
  del.classList.add("target-del");

  head.append(badge, name, up, down, del);

  const fields = document.createElement("div");
  fields.className = "target-fields";

  const kind = document.createElement("select");
  kind.className = "target-kind";
  for (const k of TARGET_KINDS) {
    const opt = document.createElement("option");
    opt.value = k.value;
    opt.textContent = k.label;
    kind.appendChild(opt);
  }
  kind.value = target.kind || "tree";
  kind.addEventListener("change", () => {
    markTargetsDirty();
    syncTargetKind(row);
  });

  const parent = document.createElement("input");
  parent.type = "text";
  parent.className = "target-parent";
  parent.placeholder = "分类树名或记忆 id";
  parent.value = target.parent || "";
  parent.setAttribute("list", "target-tree-list");
  parent.spellcheck = false;
  parent.addEventListener("input", () => {
    markTargetsDirty();
  });

  const type = document.createElement("select");
  type.className = "target-type";
  for (const t of TARGET_TYPES) {
    const opt = document.createElement("option");
    opt.value = t;
    opt.textContent = t;
    type.appendChild(opt);
  }
  type.value = target.type || "context";
  type.addEventListener("change", () => {
    markTargetsDirty();
  });

  const importance = document.createElement("select");
  importance.className = "target-importance";
  for (const im of TARGET_IMPORTANCES) {
    const opt = document.createElement("option");
    opt.value = im.value;
    opt.textContent = im.label;
    importance.appendChild(opt);
  }
  importance.value = target.importance || "important";
  importance.addEventListener("change", () => {
    markTargetsDirty();
  });

  fields.append(kind, parent, type, importance);
  row.append(head, fields);
  syncTargetKind(row);
  return row;
}

function refreshTargetBadges() {
  const rows = Array.from(targetsList.querySelectorAll(".target-row"));
  rows.forEach((row, i) => {
    const badge = row.querySelector(".target-badge");
    if (!badge) return;
    if (i === 0) {
      badge.textContent = "优先";
      badge.classList.add("primary");
      badge.title = "第 1 个 = 最优先";
    } else {
      badge.textContent = String(i + 1);
      badge.classList.remove("primary");
      badge.title = `第 ${i + 1} 个`;
    }
  });
}

function moveTargetRow(row, delta) {
  const rows = Array.from(targetsList.querySelectorAll(".target-row"));
  const idx = rows.indexOf(row);
  const next = idx + delta;
  if (idx < 0 || next < 0 || next >= rows.length) return;
  if (delta < 0) targetsList.insertBefore(row, rows[next]);
  else targetsList.insertBefore(rows[next], row);
  refreshTargetBadges();
  markTargetsDirty();
}

function removeTargetRow(row) {
  row.remove();
  refreshTargetBadges();
  markTargetsDirty();
}

function renderTargets(list) {
  targetsList.innerHTML = "";
  const items = Array.isArray(list) ? list : [];
  items.forEach((t) => {
    targetsList.appendChild(buildTargetRow(t || {}));
  });
  targetsDirty = false;
  if (!items.length) {
    const empty = document.createElement("div");
    empty.className = "browse-empty";
    empty.textContent = "还没有快捷目标，点「新增目标」加一个。";
    targetsList.appendChild(empty);
  }
  refreshTargetBadges();
}

function collectTargets() {
  const rows = Array.from(targetsList.querySelectorAll(".target-row"));
  return rows.map((row) => {
    const kind = row.querySelector(".target-kind").value;
    const isTree = kind === "tree";
    return {
      id: row.dataset.targetId || "",
      label: row.querySelector(".target-name").value.trim(),
      kind,
      parent: isTree ? row.querySelector(".target-parent").value.trim() || null : null,
      type: isTree ? row.querySelector(".target-type").value : null,
      importance: isTree ? row.querySelector(".target-importance").value : null,
    };
  });
}

async function loadTargets(force) {
  if (targetsDirty && !force) {
    setTargetsStatus("有未保存的修改");
    return;
  }
  setTargetsStatus("读取中…");
  try {
    const res = await invoke()("quick_targets_list");
    const list = Array.isArray(res?.targets) ? res.targets : [];
    if (Array.isArray(res?.taxonomy) && res.taxonomy.length) {
      targetTaxonomy = res.taxonomy;
    }
    fillTargetTreeList();
    renderTargets(list);
    setTargetsStatus(`${list.length} 个目标`);
  } catch (e) {
    targetsList.innerHTML = "";
    setTargetsStatus(`读取失败：${e?.message || e}`, "err");
  }
}

function newTargetId() {
  const base = `t${Date.now()}`;
  let id = base;
  let seq = 0;
  while (targetsList.querySelector(`.target-row[data-target-id="${id}"]`)) {
    seq += 1;
    id = `${base}-${seq}`;
  }
  return id;
}

function addTarget() {
  const rows = targetsList.querySelectorAll(".target-row");
  if (rows.length >= MAX_TARGETS) {
    setTargetsStatus(`最多 ${MAX_TARGETS} 个快捷目标`, "err");
    return;
  }
  const empty = targetsList.querySelector(".browse-empty");
  if (empty) empty.remove();
  const id = newTargetId();
  const row = buildTargetRow({
    id,
    label: `新目标 ${rows.length + 1}`,
    kind: "tree",
    parent: "",
    type: "context",
    importance: "important",
  });
  targetsList.appendChild(row);
  refreshTargetBadges();
  markTargetsDirty();
  const name = row.querySelector(".target-name");
  if (name) {
    name.focus();
    name.select();
  }
}

async function saveTargets() {
  const list = collectTargets();
  for (let i = 0; i < list.length; i++) {
    const t = list[i];
    if (!t.id) {
      setTargetsStatus(`第 ${i + 1} 个目标缺少标识`, "err");
      return;
    }
    if (!t.label) {
      setTargetsStatus(`第 ${i + 1} 个目标还没起名字`, "err");
      return;
    }
    if (t.kind === "tree" && !t.parent) {
      setTargetsStatus(`第 ${i + 1} 个目标「${t.label}」要挂节点，得先填父节点`, "err");
      return;
    }
  }

  setTargetsStatus("保存中…");
  try {
    const res = await invoke()("quick_targets_save", { targets: list });
    if (!res || res.ok !== true) {
      setTargetsStatus((res && res.message) || "保存失败", "err");
      return;
    }
    const saved = Array.isArray(res.targets) ? res.targets : list;
    if (Array.isArray(res.taxonomy) && res.taxonomy.length) {
      targetTaxonomy = res.taxonomy;
    }
    fillTargetTreeList();
    renderTargets(saved);
    setTargetsStatus(`已保存 ${saved.length} 个目标`);
  } catch (e) {
    setTargetsStatus(`保存失败：${e?.message || e}`, "err");
  }
}

async function resetTargets() {
  setTargetsStatus("恢复中…");
  try {
    const res = await invoke()("quick_targets_reset");
    if (!res || res.ok !== true) {
      setTargetsStatus((res && res.message) || "恢复失败", "err");
      return;
    }
    if (Array.isArray(res.taxonomy) && res.taxonomy.length) {
      targetTaxonomy = res.taxonomy;
    }
    fillTargetTreeList();
    renderTargets(Array.isArray(res.targets) ? res.targets : []);
    setTargetsStatus(res.message || "已恢复默认目标");
  } catch (e) {
    setTargetsStatus(`恢复失败：${e?.message || e}`, "err");
  }
}

// ---- 层一切换 ----
function updateDestination() {
  $("#write-destination").textContent = autoMode
    ? "去向：自动判断；需要指定上级时请关闭自动化。主记忆仍需核实，日记仅作参考。"
    : selectedTarget === "journal"
      ? "去向：当日活动轨迹（日记），不挂到主记忆树。"
      : "去向：主记忆 / " + (selectedParent?.title || selectedTree || "请选择上级");
}
async function searchParents() {
  const query = $("#parent-query").value.trim();
  const container = $("#parent-results"); container.replaceChildren();
  if (!query) return;
  try {
    await ensureMemoryCatalog();
    if ($("#parent-query").value.trim() !== query) return;
    const items = selectMemories(memoryCatalog || [], {query}).filter(it => !isDiary(it) && !it.superseded_by).slice(0, 12);
    for (const item of items) {
      const button = document.createElement("button"); button.type = "button"; button.className = "tree-chip";
      button.textContent = (item.title || "无标题") + " · " + item.id.slice(0, 8);
      button.addEventListener("click", () => {
        selectedParent = {id:item.id, title:item.title};
        treeGroups.querySelectorAll(".tree-chip").forEach(chip => chip.classList.remove("active"));
        container.replaceChildren(); $("#parent-query").value = ""; updateDestination();
      });
      container.append(button);
    }
    if (!items.length) container.textContent = "没有可选的有效主记忆节点。";
  } catch (error) { container.textContent = "读取上级失败：" + String(error?.message || error); }
}
async function viewSavedMemory(id) {
  browseDirty = true; catalogRevision++;
  switchView("search"); searchInput.value = id;
  searchKindValue = "quick";
  searchKind.querySelectorAll("[data-kind]").forEach(button => {
    const active = button.dataset.kind === "quick"; button.classList.toggle("active", active); button.setAttribute("aria-pressed", String(active));
  });
  $("#search-count-wrap").classList.add("hidden");
  searchInput.placeholder = "快速搜索全部记忆的标题、正文或 id";
  await runSearch();
  const card = [...searchList.querySelectorAll(".mem-item")].find(item => item.dataset.memoryId === id);
  if (card) { card.classList.add("expanded"); card.setAttribute("aria-expanded", "true"); card.scrollIntoView({block:"center"}); }
}

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
  updateDestination();
}

// ---- 自动化开关 ----
function setAuto(on) {
  autoMode = !!on;
  autoSwitch.checked = autoMode;
  const modelJudges =
    !!appConfig.llm_enabled && appConfig.automation_mode !== "local-fallback";
  updateDestination();
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

// ---- 内容梳理（P10：层次一 LLM / 层次二 agent）----
// 来源（模型 / opencode / Qoder / 关闭）由小窗下拉决定，后端存到 processor.json，
// 点选即生效（不用改 config.toml、不用重启）。
let processorState = null; // 后端 processor_status 快照
let processorBusy = false;

function processSource() {
  return processorState?.source || appConfig?.processor_source || "llm";
}

function processSourceLabel() {
  if (processorState?.source_label) return processorState.source_label;
  const s = processSource();
  if (s === "llm") return "模型";
  if (s === "off") return "关闭";
  return s;
}

/** 把后端状态同步到 appConfig + 来源按钮 */
function applyProcessorState(st) {
  if (!st) return;
  processorState = st;
  appConfig.processor_enabled = !!st.enabled;
  appConfig.processor_source = st.source;
  appConfig.processor_model = st.llm?.model || "";
  appConfig.processor_agent_kind = st.agent?.kind || "";
  appConfig.processor_agent_bin = st.agent?.bin || "";
  renderProcessorSource();
  updateProcessBtnTitle();
}

function renderProcessorSource() {
  if (!procSourceOpts) return;
  const options =
    processorState?.options?.length > 0
      ? processorState.options
      : [
          { id: "llm", label: "模型" },
          { id: "opencode", label: "opencode" },
          { id: "qoder", label: "Qoder" },
          { id: "off", label: "关闭" },
        ];
  const current = processSource();
  procSourceOpts.innerHTML = "";
  for (const opt of options) {
    const b = document.createElement("button");
    b.type = "button";
    b.className = "proc-source-btn" + (opt.id === current ? " active" : "");
    b.dataset.source = opt.id;
    b.textContent = opt.label;
    b.title = opt.hint || "";
    procSourceOpts.appendChild(b);
  }
}

function setProcHint(text, kind) {
  if (!procSourceHint) return;
  procSourceHint.textContent = text || "";
  procSourceHint.classList.toggle("ok", kind === "ok");
  procSourceHint.classList.toggle("err", kind === "err");
}

function updateProcessBtnTitle() {
  if (!processBtn) return;
  const via = processSourceLabel();
  if (!appConfig.processor_enabled) {
    processBtn.title = "梳理已关闭（可在来源里选「模型 / opencode / Qoder」）";
  } else if (via === "模型") {
    processBtn.title = "调用模型梳理内容（总结/结构化/起标题/翻译/提取/润色）";
  } else {
    processBtn.title = `调用 ${via} 梳理内容（agent headless；总结/结构化/起标题/翻译/提取/润色）`;
  }
}

/** 读取后端状态（启动 / 打开任务条时调用） */
async function loadProcessorStatus() {
  try {
    const st = await invoke()("processor_status");
    applyProcessorState(st);
    return st;
  } catch (e) {
    setProcHint(`读取梳理配置失败：${e?.message || e}`, "err");
    return null;
  }
}

/** 点来源按钮：立即切换并落盘，无需重启 */
async function selectProcessorSource(id) {
  if (processorBusy || id === processSource()) return;
  processorBusy = true;
  procSourceOpts?.classList.add("busy");
  setProcHint("正在切换…");
  try {
    const st = await invoke()("processor_set_source", { source: id });
    applyProcessorState(st);
    if (id === "off") {
      setProcHint("已关闭梳理", "ok");
    } else if (id === "llm") {
      setProcHint(
        st.llm?.configured
          ? `模型 ${st.llm.model} · ${st.llm.endpoint}`
          : "未配置模型端点，请编辑 config.toml [processor] endpoint",
        st.llm?.configured ? "ok" : "err"
      );
    } else {
      setProcHint(`正在探测 ${st.source_label}…`);
      probeProcessor();
    }
  } catch (e) {
    setProcHint(String(e?.message || e), "err");
    await loadProcessorStatus();
  } finally {
    processorBusy = false;
    procSourceOpts?.classList.remove("busy");
  }
}

/** 探测 agent CLI 是否可用（不发起梳理请求） */
async function probeProcessor() {
  try {
    const r = await invoke()("processor_probe");
    if (!r) return;
    setProcHint(r.ok ? `${r.message}（${r.version}）` : r.message, r.ok ? "ok" : "err");
  } catch (e) {
    setProcHint(String(e?.message || e), "err");
  }
}

function setProcessVisible(visible) {
  if (!processBar) return;
  processBar.classList.toggle("hidden", !visible);
  processBtn?.classList.toggle("active", visible);
}

function closeProcessBar() {
  setProcessVisible(false);
}

async function runProcess(task) {
  const content = getContent();
  if (!content) {
    setResult("warn", "内容为空", "请先输入要梳理的内容。");
    return;
  }
  if (!appConfig.processor_enabled) {
    setResult(
      "warn",
      "梳理已关闭",
      "请在「来源」里选「模型」或 opencode / Qoder 后再试（点选即生效，无需重启）。"
    );
    return;
  }
  if (content.length > 8000) {
    setResult("err", "内容过长", `已 ${content.length} 字，超过 8000 字上限；请拆分后梳理。`);
    return;
  }
  closeProcessBar();
  const via = processSourceLabel();
  setResult(
    "info",
    "正在梳理…",
    via === "模型"
      ? "正在调用模型处理内容，请稍候。"
      : `正在调用 ${via} 处理内容，请稍候（agent 冷启动可能较慢）。`
  );
  try {
    const res = await invoke()("process_memory", { content, task });
    if (!res) throw new Error("Rust 命令无返回");
    if (!res.ok) throw new Error(res.message || "梳理失败");
    // 回填输入框：三段模式切回自由文本，用户再走自动化 / 手动落库
    setInputMode("free");
    inputEl.value = res.content;
    updateCharCount();
    updateTriggerRow(false);
    const label = task === "title" ? "已起标题 ✓" : "已梳理 ✓";
    setResult("ok", `${label} · ${res.source || processSource()}`, res.content);
    inputEl.focus();
  } catch (e) {
    setResult("err", "梳理失败", String(e?.message || e));
  }
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
  if (!autoMode && selectedTarget === "main" && !selectedTree && !selectedParent) {
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
        tree: selectedTarget === "main" ? (selectedParent?.id || selectedTree) : null,
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
  catalogRevision++;
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
  if (id) {
    const view = document.createElement("button"); view.type = "button"; view.className = "mini-btn";
    view.textContent = "查看记录 · " + id.slice(0, 8);
    view.addEventListener("click", () => { void viewSavedMemory(id); });
    resultArea.appendChild(view);
  }
  // 保留保存位置和查看入口，避免用户尚未看清回执窗口就消失。

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
  $("#browse-source").addEventListener("change", renderRecent);
  let parentTimer;
  $("#parent-query").addEventListener("input", () => {
    clearTimeout(parentTimer); parentTimer = setTimeout(searchParents, 180);
  });
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
  // 梳理来源（以 processor_status 为准，避免配置读取失败时误判为未启用）
  await loadProcessorStatus();

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
  browseRefresh.addEventListener("click", () => loadRecent(true));
  browseQuery.addEventListener("input", () => {
    clearTimeout(browseTimer);
    browseTimer = setTimeout(() => { browseCollapsed.clear(); loadRecent(); }, 120);
  });
  const bindViewMode = (element, change) => element.querySelectorAll("[data-mode]").forEach(button => {
    button.addEventListener("click", () => {
      element.querySelectorAll("[data-mode]").forEach(item => { const active = item === button; item.classList.toggle("active", active); item.setAttribute("aria-pressed", String(active)); });
      change(button.dataset.mode);
    });
  });
  bindViewMode(browseViewMode, mode => { browseMode = mode; renderRecent(); });
  bindViewMode(searchViewMode, mode => { searchMode = mode; if (searchResult) renderSearch(searchResult); });
  searchKind.querySelectorAll("[data-kind]").forEach(button => button.addEventListener("click", () => {
    searchKindValue = button.dataset.kind;
    searchReq++; clearTimeout(searchTimer);
    searchKind.querySelectorAll("[data-kind]").forEach(item => { const active = item === button; item.classList.toggle("active", active); item.setAttribute("aria-pressed", String(active)); });
    $("#search-count-wrap").classList.toggle("hidden", searchKindValue === "quick");
    searchInput.placeholder = searchKindValue === "quick" ? "快速搜索全部记忆的标题、正文或 id" : "描述要查找的内容，或粘贴 id";
    runSearch();
  }));
  searchInput.addEventListener("input", () => {
    clearTimeout(searchTimer);
    if (searchKindValue === "quick") searchTimer = setTimeout(runSearch, 120);
  });
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

  // 快捷目标
  targetsAdd.addEventListener("click", addTarget);
  targetsReset.addEventListener("click", resetTargets);
  targetsSave.addEventListener("click", saveTargets);

  graphSearch.addEventListener("input", renderGraphSearch);
  graphSearch.addEventListener("focus", renderGraphSearch);
  graphSearch.addEventListener("keydown", event => {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      graphSearch.value = "";
      renderGraphSearch();
    } else if (event.key === "Enter" || event.key === "ArrowDown") {
      event.preventDefault();
      const first = graphSearchResults.querySelector("button");
      if (event.key === "Enter") first?.click();
      else first?.focus();
    }
  });
  graphSearchResults.addEventListener("keydown", event => {
    const buttons = [...graphSearchResults.querySelectorAll("button")];
    const index = buttons.indexOf(document.activeElement);
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      graphSearch.focus();
      graphSearchResults.classList.add("hidden");
    }
    else if (["ArrowDown", "ArrowUp"].includes(event.key)) {
      event.preventDefault();
      buttons[Math.max(0, Math.min(buttons.length - 1, index + (event.key === "ArrowDown" ? 1 : -1)))]?.focus();
    }
  });
  document.addEventListener("pointerdown", event => {
    if (!event.target.closest(".graph-search-field")) graphSearchResults.classList.add("hidden");
  });
  graphClearSelection.addEventListener("click", () => graphView?.selectNode(null));

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
  checkUpdate(false);

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

  // 层一
  document.querySelectorAll(".seg-btn").forEach((b) =>
    b.addEventListener("click", () => setTarget(b.dataset.target))
  );

  // 内容梳理（P10）：开关任务条 + 各任务点击
  processBtn?.addEventListener("click", async () => {
    if (processBar?.classList.contains("hidden")) {
      setProcessVisible(true);
      await loadProcessorStatus();
      if (!appConfig.processor_enabled) {
        setProcHint("梳理已关闭：选一个来源即可启用", "err");
      }
    } else {
      closeProcessBar();
    }
  });
  procSourceOpts?.addEventListener("click", (e) => {
    const b = e.target.closest(".proc-source-btn");
    if (b) selectProcessorSource(b.dataset.source);
  });
  processBar?.addEventListener("click", (e) => {
    const btn = e.target.closest(".process-task");
    if (btn) runProcess(btn.dataset.task);
  });
  updateProcessBtnTitle();

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

  try {
    await listen()("selection-view-memory", ev => {
      const id = String(ev?.payload || "");
      if (/^[a-f0-9-]{8,}$/i.test(id)) void viewSavedMemory(id);
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
    } else {
      runtimeDot.classList.add("offline");
      runtimeDot.classList.remove("online");
      runtimeDot.title = st?.message || "rsrs runtime 不可达 — 由宿主启动服务后自动恢复";
      syncBadge.classList.add("hidden");
    }
  } catch (_) {
    runtimeDot.classList.add("offline");
    runtimeDot.classList.remove("online");
    runtimeDot.title = "rsrs runtime 不可达";
    syncBadge.classList.add("hidden");
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
  loadTargets,
  saveTargets,
  resetTargets,
  graph: () => graphView,
  loadGraphSnapshot,
  rebuildGraphLayout,
  pollGraph,
  checkUpdate,
  installUpdate,
  runProcess,
};
