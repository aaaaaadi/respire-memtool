// memTools 扩展选项：保存端口/令牌，测试 /health，并编辑「快捷目标」列表。
// 快捷目标的权威数据在 memTools 侧（GET/PUT /targets），本页只是编辑器。

const $ = (s) => document.querySelector(s);
const statusEl = $("#status");
const targetsStatusEl = $("#targets-status");

const DEFAULTS = { port: 17650 };
const MAX_TARGETS = 8;

const KIND_OPTIONS = [
  ["journal", "日记"],
  ["tree", "挂到节点"],
  ["auto", "自动判档"],
];
const TYPE_OPTIONS = [
  "context",
  "decision",
  "preference",
  "task",
  "emotion",
  "time",
  "skill",
];
const IMPORTANCE_OPTIONS = ["important", "trivial"];

// 与后端一致的默认目标（恢复默认时整表写回）
const DEFAULT_TARGETS = [
  {
    id: "journal",
    label: "日记",
    kind: "journal",
    parent: null,
    type: null,
    importance: "trivial",
  },
  {
    id: "auto",
    label: "自动判档",
    kind: "auto",
    parent: null,
    type: null,
    importance: null,
  },
];

let targets = [];
let taxonomy = [];
let editable = false;

function setStatus(text, kind) {
  statusEl.textContent = text;
  statusEl.className = kind || "";
}

function setTargetsStatus(text, kind) {
  targetsStatusEl.textContent = text;
  targetsStatusEl.className = kind || "";
}

function esc(value) {
  return String(value ?? "").replace(
    /[&<>"']/g,
    (ch) =>
      ({
        "&": "&amp;",
        "<": "&lt;",
        ">": "&gt;",
        '"': "&quot;",
        "'": "&#39;",
      })[ch],
  );
}

function normalizeTarget(raw) {
  const kind = ["journal", "tree", "auto"].includes(raw?.kind)
    ? raw.kind
    : "auto";
  return {
    id: String(raw?.id || ""),
    label: String(raw?.label || "").slice(0, 24),
    kind,
    parent: raw?.parent ? String(raw.parent) : null,
    type: raw?.type ? String(raw.type) : null,
    importance: raw?.importance ? String(raw.importance) : null,
  };
}

function currentCred() {
  const port = Number($("#port").value) || DEFAULTS.port;
  const token = $("#token").value.trim();
  return { port, token };
}

async function apiRequest(path, method = "GET", body = null) {
  const { port, token } = currentCred();
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 10000);
  try {
    const resp = await fetch(`http://127.0.0.1:${port}${path}`, {
      method,
      headers: {
        "Content-Type": "application/json",
        "X-MemTools-Token": token,
      },
      body: body ? JSON.stringify(body) : undefined,
      signal: controller.signal,
    });
    const data = await resp.json().catch(() => ({}));
    return { status: resp.status, ok: resp.ok, data };
  } catch (e) {
    return { status: 0, ok: false, data: {}, error: e };
  } finally {
    clearTimeout(timer);
  }
}

async function load() {
  const got = await chrome.storage.local.get(["port", "token"]);
  if (got.port) $("#port").value = String(got.port);
  if (typeof got.token === "string") $("#token").value = got.token;
  setEditable(false);
  await loadTargets();
}

async function save() {
  const port = Number($("#port").value);
  const token = $("#token").value.trim();
  if (!Number.isInteger(port) || port < 1 || port > 65535) {
    setStatus("端口无效（1-65535）", "err");
    return;
  }
  await chrome.storage.local.set({ port, token });
  setStatus("已保存", "ok");
  await loadTargets();
}

async function test() {
  const port = Number($("#port").value);
  const token = $("#token").value.trim();
  if (!token) {
    setStatus("请先填入令牌", "err");
    return;
  }
  setStatus("测试中…");
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 5000);
  try {
    const resp = await fetch(`http://127.0.0.1:${port}/health`, {
      headers: { "X-MemTools-Token": token },
      signal: controller.signal,
    });
    if (resp.status === 401) {
      setStatus("令牌无效（HTTP 401）", "err");
      return;
    }
    const data = await resp.json().catch(() => ({}));
    if (resp.ok && data.ok) {
      setStatus(`连接成功：${data.name} v${data.version}`, "ok");
      await chrome.storage.local.set({ port, token });
      await loadTargets();
    } else {
      setStatus(`连接失败：HTTP ${resp.status}`, "err");
    }
  } catch (e) {
    setStatus(
      e?.name === "AbortError"
        ? "连接超时：请确认 memTools 正在运行"
        : `无法连接 127.0.0.1:${port}：请确认 memTools 正在运行`,
      "err",
    );
  } finally {
    clearTimeout(timer);
  }
}

// 打开 memTools 主窗（不带文字）
async function openMemTools() {
  setStatus("正在打开 memTools…");
  const res = await apiRequest("/open", "POST", {});
  if (res.status === 401) {
    setStatus("令牌无效（HTTP 401）", "err");
    return;
  }
  if (!res.ok) {
    setStatus(res.data?.message || `打开失败：HTTP ${res.status}`, "err");
    return;
  }
  setStatus(res.data?.message || "已打开 memTools", "ok");
}

// 读快捷目标：不可达 / 令牌无效时禁用编辑，但端口令牌设置仍可用
async function loadTargets() {
  const { port } = currentCred();
  setTargetsStatus("读取快捷目标…");
  const res = await apiRequest("/targets", "GET");
  if (res.status === 401) {
    setTargetsStatus("令牌无效（HTTP 401）", "err");
    setEditable(false);
    return;
  }
  if (!res.ok) {
    const why =
      res.error?.name === "AbortError"
        ? "连接超时"
        : `无法连接 127.0.0.1:${port}`;
    setTargetsStatus(`请先启动 memTools（${why}）`, "err");
    setEditable(false);
    return;
  }
  const list = Array.isArray(res.data.targets) ? res.data.targets : [];
  targets = list.map(normalizeTarget).filter((t) => t.id);
  taxonomy = Array.isArray(res.data.taxonomy) ? res.data.taxonomy : [];
  renderTaxonomy();
  renderTargets();
  setEditable(true);
  setTargetsStatus(`已载入 ${targets.length} 个目标`, "ok");
}

function setEditable(on) {
  editable = on;
  document
    .querySelectorAll(
      "#targets-card input, #targets-card select, #targets-card button",
    )
    .forEach((el) => {
      el.disabled = !on;
    });
}

function renderTaxonomy() {
  const list = $("#taxonomy-list");
  list.innerHTML = "";
  taxonomy.forEach((item) => {
    const opt = document.createElement("option");
    opt.value = item?.name || "";
    if (item?.domain) opt.label = item.domain;
    list.appendChild(opt);
  });
}

function rowHtml(t, index) {
  const kindOptions = KIND_OPTIONS.map(
    ([value, label]) =>
      `<option value="${value}"${value === t.kind ? " selected" : ""}>${label}</option>`,
  ).join("");
  const typeOptions = TYPE_OPTIONS.map(
    (value) =>
      `<option value="${value}"${value === t.type ? " selected" : ""}>${value}</option>`,
  ).join("");
  const importanceOptions = IMPORTANCE_OPTIONS.map(
    (value) =>
      `<option value="${value}"${value === t.importance ? " selected" : ""}>${value}</option>`,
  ).join("");
  return `
    <div class="target-row${t.kind === "tree" ? " is-tree" : ""}" data-index="${index}">
      <div class="head">
        <span class="idx">${index + 1}</span>
        <input class="t-label" type="text" maxlength="24" placeholder="名称（≤24 字）" value="${esc(t.label)}" />
        <button class="small t-up" title="上移" type="button">↑</button>
        <button class="small t-down" title="下移" type="button">↓</button>
        <button class="small t-del" title="删除" type="button">×</button>
      </div>
      <div class="fields">
        <label>
          <span>类型</span>
          <select class="t-kind">${kindOptions}</select>
        </label>
        <label class="t-tree-only">
          <span>父节点（分类树名或记忆标题）</span>
          <input class="t-parent" type="text" list="taxonomy-list" placeholder="如 AI-domain" value="${esc(t.parent || "")}" />
        </label>
        <label class="t-tree-only">
          <span>性质</span>
          <select class="t-type">${typeOptions}</select>
        </label>
        <label class="t-tree-only">
          <span>重要度</span>
          <select class="t-importance">${importanceOptions}</select>
        </label>
      </div>
    </div>`;
}

function renderTargets() {
  const box = $("#targets-list");
  box.innerHTML = "";
  if (!targets.length) {
    const empty = document.createElement("div");
    empty.className = "empty";
    empty.textContent = "还没有快捷目标，点「新增目标」添加，或「恢复默认」。";
    box.appendChild(empty);
    return;
  }
  targets.forEach((t, index) => {
    const holder = document.createElement("div");
    holder.innerHTML = rowHtml(t, index);
    const row = holder.firstElementChild;
    bindRow(row, index);
    box.appendChild(row);
  });
}

function bindRow(row, index) {
  const t = targets[index];
  const label = row.querySelector(".t-label");
  const kind = row.querySelector(".t-kind");
  const parent = row.querySelector(".t-parent");
  const type = row.querySelector(".t-type");
  const importance = row.querySelector(".t-importance");

  label.addEventListener("input", () => {
    t.label = label.value;
  });
  kind.addEventListener("change", () => {
    t.kind = kind.value;
    if (t.kind === "tree") {
      t.type = t.type || "context";
      t.importance = t.importance || "important";
    }
    renderTargets();
  });
  parent.addEventListener("input", () => {
    t.parent = parent.value;
  });
  type.addEventListener("change", () => {
    t.type = type.value;
  });
  importance.addEventListener("change", () => {
    t.importance = importance.value;
  });
  row.querySelector(".t-up").addEventListener("click", () => {
    moveTarget(index, -1);
  });
  row.querySelector(".t-down").addEventListener("click", () => {
    moveTarget(index, 1);
  });
  row.querySelector(".t-del").addEventListener("click", () => {
    targets.splice(index, 1);
    renderTargets();
    setTargetsStatus(`已删除一个目标，剩 ${targets.length} 个（记得保存）`);
  });
}

function moveTarget(index, delta) {
  const next = index + delta;
  if (next < 0 || next >= targets.length) return;
  const [item] = targets.splice(index, 1);
  targets.splice(next, 0, item);
  renderTargets();
}

function addTarget() {
  if (!editable) {
    setTargetsStatus("请先启动 memTools", "err");
    return;
  }
  if (targets.length >= MAX_TARGETS) {
    setTargetsStatus(`最多 ${MAX_TARGETS} 个快捷目标`, "err");
    return;
  }
  targets.push({
    id: `t${Date.now()}`,
    label: `新目标 ${targets.length + 1}`,
    kind: "tree",
    parent: "",
    type: "context",
    importance: "important",
  });
  renderTargets();
  setTargetsStatus("已新增一行，填好后点「保存目标」");
}

// 送给后端的形状：parent / type / importance 仅对 kind=tree 有意义
function toPayload(list) {
  return list.map((t) => {
    const item = { id: t.id, label: t.label, kind: t.kind };
    if (t.kind === "tree") {
      item.parent = String(t.parent || "").trim();
      item.type = t.type || "context";
      item.importance = t.importance || "important";
    } else {
      item.parent = null;
      item.type = null;
      item.importance = null;
    }
    return item;
  });
}

function validateTargets(list) {
  if (list.length > MAX_TARGETS) {
    return `最多 ${MAX_TARGETS} 个快捷目标`;
  }
  for (let i = 0; i < list.length; i++) {
    const t = list[i];
    if (!String(t.label || "").trim()) {
      return `第 ${i + 1} 个目标还没填名称`;
    }
    if (t.kind === "tree" && !String(t.parent || "").trim()) {
      return `第 ${i + 1} 个目标「${t.label}」是「挂到节点」，需要填写父节点`;
    }
  }
  return null;
}

async function saveTargets(list, okPrefix) {
  const err = validateTargets(list);
  if (err) {
    setTargetsStatus(err, "err");
    return;
  }
  setTargetsStatus("保存中…");
  const res = await apiRequest("/targets", "PUT", { targets: toPayload(list) });
  if (res.status === 401) {
    setTargetsStatus("令牌无效（HTTP 401）", "err");
    setEditable(false);
    return;
  }
  if (!res.ok) {
    setTargetsStatus(res.data?.message || `保存失败：HTTP ${res.status}`, "err");
    return;
  }
  if (Array.isArray(res.data.targets)) {
    targets = res.data.targets.map(normalizeTarget).filter((t) => t.id);
  }
  if (Array.isArray(res.data.taxonomy)) {
    taxonomy = res.data.taxonomy;
    renderTaxonomy();
  }
  renderTargets();
  setEditable(true);
  setTargetsStatus(
    `${okPrefix || "已保存"} ${targets.length} 个目标`,
    "ok",
  );
  // 让后台 Service Worker 立即重建右键菜单
  try {
    await chrome.runtime.sendMessage({ type: "memtools:targets-updated" });
  } catch (_) {
    // 后台未就绪时忽略，下一次刷新会同步
  }
}

async function resetTargets() {
  if (!editable) {
    setTargetsStatus("请先启动 memTools", "err");
    return;
  }
  const defaults = DEFAULT_TARGETS.map((t) => ({ ...t }));
  await saveTargets(defaults, "已恢复默认");
}

$("#save").addEventListener("click", save);
$("#test").addEventListener("click", test);
$("#open-memtools").addEventListener("click", openMemTools);
$("#add-target").addEventListener("click", addTarget);
$("#reset-targets").addEventListener("click", resetTargets);
$("#save-targets").addEventListener("click", () => saveTargets(targets));
load();
