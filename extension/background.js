// memTools 记忆捕获 — MV3 后台 Service Worker
// 右键菜单 → 取选区 → POST 到本机 memTools 端点（127.0.0.1，带令牌）。

const MENU_ROOT = "memtools-save";
const MENU_JOURNAL = "memtools:journal";
const MENU_AUTO = "memtools:auto";

const DEFAULTS = { port: 17650, token: "" };

// 每次启动重建菜单，避免升级后残留旧项
function installMenus() {
  chrome.contextMenus.removeAll(() => {
    chrome.contextMenus.create({
      id: MENU_ROOT,
      title: "保存到 memTools",
      contexts: ["selection"],
    });
    chrome.contextMenus.create({
      id: MENU_JOURNAL,
      parentId: MENU_ROOT,
      title: "存为日记（trivial）",
      contexts: ["selection"],
    });
    chrome.contextMenus.create({
      id: MENU_AUTO,
      parentId: MENU_ROOT,
      title: "存到主区（自动判重 / 挂树）",
      contexts: ["selection"],
    });
  });
}

chrome.runtime.onInstalled.addListener(installMenus);
chrome.runtime.onStartup.addListener(installMenus);
// Service Worker 每次启动都重建一次：避免“先加载扩展、后改代码”导致菜单缺失
installMenus();

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

async function getSettings() {
  const got = await chrome.storage.local.get(["port", "token"]);
  return {
    port: Number(got.port) || DEFAULTS.port,
    token: typeof got.token === "string" ? got.token.trim() : "",
  };
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

async function save({ text, mode, url, pageTitle }) {
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
  const timer = setTimeout(() => controller.abort(), 45000);
  // MV3 Service Worker 空闲 30s 会被回收；rsrs 写入可能较慢，用扩展 API 续命
  const keepAlive = setInterval(() => chrome.runtime.getPlatformInfo(() => {}), 20000);
  try {
    const resp = await fetch(`http://127.0.0.1:${port}/remember`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "X-MemTools-Token": token,
      },
      body: JSON.stringify({
        text,
        mode,
        url,
        page_title: pageTitle,
      }),
      signal: controller.signal,
    });
    const data = await resp.json().catch(() => ({}));
    if (resp.status === 401) {
      notify("memTools 令牌无效", "请重新复制 extension_token.txt 到扩展选项。");
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
    const warn = data.needs_confirm
      ? "\n⚠ 疑似重复，已挂到相似条目下，请回小窗核对。"
      : "";
    notify(`已保存到${where}`, `${data.title || "（无标题）"}${id}${warn}`);
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

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  if (info.menuItemId !== MENU_JOURNAL && info.menuItemId !== MENU_AUTO) return;
  const text = await readSelection(tab?.id, info.selectionText);
  if (!text) {
    notify("没有取到选中文字", "请先选中一段文字再右键。");
    return;
  }
  await save({
    text,
    mode: info.menuItemId === MENU_JOURNAL ? "journal" : "auto",
    url: tab?.url || "",
    pageTitle: tab?.title || "",
  });
});
