importScripts("settings.js");

// memTools 记忆捕获 — MV3 后台 Service Worker
// 右键菜单 → 取选区 → POST 到本机 memTools 端点（127.0.0.1，带令牌）。
// 快捷目标（P9）：菜单项按 memTools 侧配置的「快捷目标」动态生成；
// chrome.storage.local 只留一份缓存，memTools 不可达时用于降级显示，权威数据始终在 memTools 侧。

const MENU_ROOT = "memtools-save";
const MENU_EDIT = "memtools:edit";
const MENU_OPEN = "memtools:open";
const MENU_SEP = "memtools:sep";
const MENU_PROCESS = "memtools:process";
const MENU_TARGET_PREFIX = "memtools:target:";

const DEFAULTS = { port: 17650, token: "" };

// 降级列表：memTools 不可达且本地无缓存时使用（权威数据在 memTools 侧）
const FALLBACK_TARGETS = [
  {
    id: "journal",
    label: "存为日记",
    kind: "journal",
    parent: null,
    type: null,
    importance: "trivial",
  },
  {
    id: "auto",
    label: "存到主区（自动判档）",
    kind: "auto",
    parent: null,
    type: null,
    importance: null,
  },
];

const REFRESH_ALARM = "memtools-refresh";
const REFRESH_MINUTES = 10;
const AUTH_WARN_GAP_MS = 60000;

let currentTargets = null;
let lastAuthWarnAt = 0;
let menuQueue = Promise.resolve();

// 每次启动重建菜单，避免升级后残留旧项
function buildMenus(targets) {
  return new Promise((resolve) => {
    chrome.contextMenus.removeAll(() => {
      createMenu({
        id: MENU_ROOT,
        title: "保存到 memTools",
        contexts: ["selection"],
      });
      // 第 1 个 = 最优先，加 ★ 标记
      targets.forEach((target, index) => {
        createMenu({
          id: `${MENU_TARGET_PREFIX}${target.id}`,
          parentId: MENU_ROOT,
          title: index === 0 ? `★ ${target.label}` : target.label,
          contexts: ["selection"],
        });
      });
      createMenu({
        id: MENU_SEP,
        parentId: MENU_ROOT,
        type: "separator",
      });
      createMenu({
        id: MENU_PROCESS,
        parentId: MENU_ROOT,
        title: "梳理后保存（AI 总结）",
        contexts: ["selection"],
      });
      createMenu({
        id: MENU_EDIT,
        parentId: MENU_ROOT,
        title: "在 memTools 中选择…",
        contexts: ["selection"],
      });
      createMenu({
        id: MENU_OPEN,
        parentId: MENU_ROOT,
        title: "打开 memTools",
        contexts: ["selection"],
      });
      resolve();
    });
  });
}

function createMenu(props) {
  try {
    chrome.contextMenus.create(props, () => void chrome.runtime.lastError);
  } catch (_) {
    // 重复 id 等异常不应打断后续菜单项
  }
}

function installMenus() {
  menuQueue = menuQueue
    .then(() => getEffectiveTargets())
    .then((targets) => buildMenus(targets))
    .catch(() => {});
  return menuQueue;
}

chrome.runtime.onInstalled.addListener(async () => {
  await getSettings({ refresh: true });
  installMenus();
});
chrome.runtime.onStartup.addListener(installMenus);
// Service Worker 每次启动都重建一次：避免“先加载扩展、后改代码”导致菜单缺失
setupRefreshAlarm();
installMenus();

// 每 10 分钟拉一次快捷目标（浏览器重启后 alarms 仍在，get 判重避免重置周期）
function setupRefreshAlarm() {
  try {
    chrome.alarms.get(REFRESH_ALARM, (alarm) => {
      if (!alarm) {
        chrome.alarms.create(REFRESH_ALARM, {
          periodInMinutes: REFRESH_MINUTES,
        });
      }
    });
  } catch (_) {
    // 权限缺失时静默降级
  }
}

chrome.alarms.onAlarm.addListener((alarm) => {
  if (alarm.name === REFRESH_ALARM) refreshTargets();
});

// 选项页保存目标后通知后台重建菜单
chrome.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
  if (msg && msg.type === "memtools:targets-updated") {
    refreshTargets().then((result) => sendResponse({ ok: !!result.ok }));
    return true;
  }
  return false;
});

// 点工具栏图标 → 打开扩展选项（没有 popup）
chrome.action.onClicked.addListener(() => chrome.runtime.openOptionsPage());

function notify(title, message) {
  chrome.notifications.create(
    `memtools-${Date.now()}`,
    {
      type: "basic",
      iconUrl: "icon128.png",
      title,
      message: String(message || "").slice(0, 300),
    },
    () => void chrome.runtime.lastError,
  );
}


function normalizeTarget(raw) {
  const kind = ["journal", "tree", "auto"].includes(raw?.kind)
    ? raw.kind
    : "auto";
  return {
    id: String(raw?.id || ""),
    label: String(raw?.label || "").trim().slice(0, 24),
    kind,
    parent: raw?.parent ? String(raw.parent) : null,
    type: raw?.type ? String(raw.type) : null,
    importance: raw?.importance ? String(raw.importance) : null,
  };
}

function normalizeList(list) {
  if (!Array.isArray(list)) return [];
  return list.map(normalizeTarget).filter((t) => t.id);
}

// 本地缓存（仅降级用，权威数据在 memTools 侧）
async function getCachedTargets() {
  const got = await chrome.storage.local.get(["targets"]);
  return normalizeList(got.targets);
}

async function getEffectiveTargets() {
  if (currentTargets && currentTargets.length) return currentTargets;
  const cached = await getCachedTargets();
  currentTargets = cached.length
    ? cached
    : FALLBACK_TARGETS.map(normalizeTarget);
  return currentTargets;
}

// 令牌无效只提醒一次，别刷屏
function warnAuthOnce() {
  const now = Date.now();
  if (now - lastAuthWarnAt < AUTH_WARN_GAP_MS) return;
  lastAuthWarnAt = now;
  notify("memTools 令牌无效", "请重新复制 extension_token.txt 到扩展选项。");
}

// 拉取快捷目标：成功则写缓存并重建菜单；401 不缓存；网络失败保留旧缓存
async function refreshTargets() {
  const { port, token } = await getSettings();
  if (!token) return { ok: false, reason: "no-token" };

  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 15000);
  // MV3 Service Worker 空闲 30s 会被回收；用扩展 API 续命
  const keepAlive = setInterval(
    () => chrome.runtime.getPlatformInfo(() => {}),
    20000,
  );
  try {
    const resp = await fetch(`http://127.0.0.1:${port}/targets`, {
      headers: { "X-MemTools-Token": token },
      signal: controller.signal,
    });
    if (resp.status === 401) {
      warnAuthOnce();
      return { ok: false, reason: "unauthorized" };
    }
    if (!resp.ok) return { ok: false, reason: "http", status: resp.status };
    const data = await resp.json().catch(() => ({}));
    const list = normalizeList(data.targets);
    if (!list.length) return { ok: false, reason: "empty" };
    currentTargets = list;
    await chrome.storage.local.set({ targets: list });
    await installMenus();
    return { ok: true, targets: list };
  } catch (e) {
    // 不可达：保留旧缓存与当前菜单
    return { ok: false, reason: "unreachable", error: String(e?.message || e) };
  } finally {
    clearTimeout(timer);
    clearInterval(keepAlive);
  }
}

// 优先用 scripting 取完整选区；受限页面回退到 info.selectionText
async function readSelection(tabId, fallback) {
  if (!tabId) return fallback || "";
  try {
    const [res] = await chrome.scripting.executeScript({
      target: { tabId },
      func: () => window.getSelection?.().toString() || "",
    });
    const text = (res?.result || "").trim();
    if (text) return text;
  } catch (_) {
    // chrome://、扩展商店等页面无法注入，回退
  }
  return (fallback || "").trim();
}

// 令牌为空 → 沿用既有行为：提醒 + 打开选项页
async function save({ text, mode, targetId, url, pageTitle, process }) {
  const { port, token } = await getSettings();
  if (!token) {
    notify(
      "memTools 未配置令牌",
      "请在 memTools 的浏览器扩展面板点击安装 / 配对，再加载生成的扩展目录。",
    );
    chrome.runtime.openOptionsPage();
    return;
  }

  const controller = new AbortController();
  // AI 梳理可能比单纯写入慢，放宽到 120s
  const timer = setTimeout(() => controller.abort(), process ? 120000 : 45000);
  // MV3 Service Worker 空闲 30s 会被回收；rsrs 写入可能较慢，用扩展 API 续命
  const keepAlive = setInterval(() => chrome.runtime.getPlatformInfo(() => {}), 20000);
  try {
    const body = { text, url, page_title: pageTitle };
    if (targetId) body.target_id = targetId;
    else if (mode) body.mode = mode;
    if (process) body.process = process;
    const resp = await fetch(`http://127.0.0.1:${port}/remember`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "X-MemTools-Token": token,
      },
      body: JSON.stringify(body),
      signal: controller.signal,
    });
    const data = await resp.json().catch(() => ({}));
    if (resp.status === 401) {
      notify("memTools 令牌无效", "请在 memTools 点击安装 / 配对，然后在 Chrome 扩展页重新加载此扩展。");
      return;
    }
    if (!resp.ok || data.ok === false) {
      notify("memTools 保存失败", data.message || `HTTP ${resp.status}`);
      return;
    }
    const where =
      data.branch === "journal"
        ? "日记"
        : data.branch === "attach"
          ? "已有记忆之下"
          : "主区分类树";
    const id = data.id ? `\nid: ${String(data.id).slice(0, 8)}` : "";
    const parent = data.parent ? `\n挂点：${data.parent}` : "";
    const warn = data.needs_confirm
      ? "\n⚠ 疑似重复，已挂到相似条目下，请回小窗核对。"
      : "";
    const prefix = process ? "（AI 梳理）" : "";
    notify(
      `已保存到${where}${prefix}`,
      `${data.title || "（无标题）"}${id}${parent}${warn}`,
    );
  } catch (e) {
    const msg =
      e?.name === "AbortError"
        ? "请求超时：rsrs 写入较慢，请稍后在 memTools 中确认。"
        : `无法连接 memTools（127.0.0.1:${port}）：请确认应用正在运行。`;
    notify("memTools 不可达", msg);
  } finally {
    clearTimeout(timer);
    clearInterval(keepAlive);
  }
}

// 唤醒 memTools：带 text 时把选区文字预填进主窗
async function openMemTools({ text, url, pageTitle } = {}) {
  const { port, token } = await getSettings();
  if (!token) {
    notify(
      "memTools 未配置令牌",
      "请打开扩展选项，粘贴 memTools 配置目录下 extension_token.txt 的内容。",
    );
    chrome.runtime.openOptionsPage();
    return;
  }

  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 15000);
  const keepAlive = setInterval(() => chrome.runtime.getPlatformInfo(() => {}), 20000);
  try {
    const body = {};
    if (text) body.text = text;
    if (url) body.url = url;
    if (pageTitle) body.page_title = pageTitle;
    const resp = await fetch(`http://127.0.0.1:${port}/open`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "X-MemTools-Token": token,
      },
      body: JSON.stringify(body),
      signal: controller.signal,
    });
    const data = await resp.json().catch(() => ({}));
    if (resp.status === 401) {
      notify("memTools 令牌无效", "请重新复制 extension_token.txt 到扩展选项。");
      return;
    }
    if (!resp.ok || data.ok === false) {
      notify("memTools 打开失败", data.message || `HTTP ${resp.status}`);
      return;
    }
    notify(
      "已打开 memTools",
      text
        ? "已把选中文字送入 memTools 主窗。"
        : "已唤醒 memTools 主窗。",
    );
  } catch (e) {
    const msg =
      e?.name === "AbortError"
        ? "请求超时：请确认 memTools 正在运行。"
        : `无法连接 memTools（127.0.0.1:${port}）：请确认应用正在运行。`;
    notify("memTools 不可达", msg);
  } finally {
    clearTimeout(timer);
    clearInterval(keepAlive);
  }
}

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  const menuId = String(info.menuItemId || "");

  if (menuId === MENU_OPEN) {
    await openMemTools({});
    return;
  }

  if (menuId === MENU_PROCESS) {
    const text = await readSelection(tab?.id, info.selectionText);
    if (!text) {
      notify("没有取到选中文字", "请先选中一段文字再右键。");
      return;
    }
    await save({
      text,
      mode: "auto",
      process: "summarize",
      url: tab?.url || "",
      pageTitle: tab?.title || "",
    });
    return;
  }

  if (menuId === MENU_EDIT) {
    const text = await readSelection(tab?.id, info.selectionText);
    await openMemTools({
      text,
      url: tab?.url || "",
      pageTitle: tab?.title || "",
    });
    return;
  }

  if (menuId.startsWith(MENU_TARGET_PREFIX)) {
    const targetId = menuId.slice(MENU_TARGET_PREFIX.length);
    const text = await readSelection(tab?.id, info.selectionText);
    if (!text) {
      notify("没有取到选中文字", "请先选中一段文字再右键。");
      return;
    }
    await save({
      text,
      targetId,
      url: tab?.url || "",
      pageTitle: tab?.title || "",
    });
  }
});
