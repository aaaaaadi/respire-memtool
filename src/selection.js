/* ============================================================
   memTools 划词悬浮窗
   收起：40x40 图标；展开：280x168 面板（窗口尺寸由 Rust 调整）
   ============================================================ */

const state = {
  text: "",
  source: "",
  expanded: false,
  busy: false,
  opening: false,
  hideTimer: 0,
  pendingError: "",
  epoch: 0,
};

const $ = (id) => document.getElementById(id);

function tauriApi() {
  const api = window.__TAURI__;
  if (!api || !api.core || typeof api.core.invoke !== "function") return null;
  return api;
}

async function call(cmd, args) {
  const api = tauriApi();
  if (!api) throw new Error("请在 memTools 中打开");
  return api.core.invoke(cmd, args);
}

async function listenEvent(name, handler) {
  const api = tauriApi();
  if (!api || !api.event || typeof api.event.listen !== "function") return null;
  try {
    return await api.event.listen(name, (event) => {
      try {
        handler(event);
      } catch (_) {
        /* 界面处理失败时不抛出，避免控制台报错 */
      }
    });
  } catch (_) {
    return null;
  }
}

function messageOf(err) {
  if (!err) return "未知错误";
  const raw = typeof err === "string" ? err : err.message || String(err);
  const text = String(raw).replace(/^Error:\s*/i, "").trim();
  return text || "操作失败";
}

function countChars(text) {
  return Array.from(text || "").length;
}

function clearHideTimer() {
  if (state.hideTimer) {
    window.clearTimeout(state.hideTimer);
    state.hideTimer = 0;
  }
}

function setStatus(message, kind) {
  const el = $("status");
  if (!el) return;
  const text = message || "";
  el.textContent = text;
  el.className = "status" + (text ? " show" : "") + (kind ? " " + kind : "");
}

function updateButtons() {
  const hasText = state.text.trim().length > 0;
  const journal = $("btn-journal");
  const edit = $("btn-edit");
  if (journal) {
    journal.disabled = state.busy || !hasText;
    journal.textContent = state.busy ? "保存中…" : "存为日记";
  }
  if (edit) edit.disabled = state.busy || !hasText;
}

function renderPayload() {
  const text = state.text || "";
  const source = (state.source || "").trim();
  const textEl = $("preview-text");
  const sourceEl = $("preview-source");
  const countEl = $("char-count");

  if (textEl) {
    if (text.trim()) {
      textEl.textContent = text;
      textEl.classList.remove("empty");
    } else {
      textEl.textContent = "未获取到选中文字";
      textEl.classList.add("empty");
    }
  }

  if (sourceEl) {
    sourceEl.textContent = source ? "来自 " + source : "";
    sourceEl.title = source;
  }

  if (countEl) countEl.textContent = countChars(text) + " 字";

  updateButtons();
}

function applyPayload(payload) {
  const text = payload && typeof payload.text === "string" ? payload.text : "";
  const source = payload && typeof payload.source === "string" ? payload.source : "";
  state.text = text;
  state.source = source;
  renderPayload();
}

function enterCollapsed() {
  state.epoch += 1;
  clearHideTimer();
  state.expanded = false;
  state.busy = false;
  state.opening = false;
  document.body.classList.add("collapsed");
  document.body.classList.remove("expanded");
  const panel = $("panel");
  const orb = $("orb");
  if (panel) panel.setAttribute("aria-hidden", "true");
  if (orb) orb.setAttribute("aria-hidden", "false");
  setStatus("", "");
  updateButtons();
}

function enterExpanded() {
  state.epoch += 1;
  state.expanded = true;
  state.busy = false;
  document.body.classList.remove("collapsed");
  document.body.classList.add("expanded");
  const panel = $("panel");
  const orb = $("orb");
  if (panel) panel.setAttribute("aria-hidden", "false");
  if (orb) orb.setAttribute("aria-hidden", "true");
  setStatus("", "");
  renderPayload();
}

function pulseOrb() {
  const orb = $("orb");
  if (!orb) return;
  orb.classList.remove("pulse");
  void orb.offsetWidth;
  orb.classList.add("pulse");
}

async function pullPayload(epoch) {
  try {
    const payload = await call("selection_payload");
    if (epoch !== state.epoch) return "";
    applyPayload(payload);
    return "";
  } catch (err) {
    if (epoch !== state.epoch) return "";
    return "读取选中文字失败：" + messageOf(err);
  }
}

function handleShow(event) {
  state.epoch += 1;
  const payload = event && event.payload;
  state.pendingError = "";
  applyPayload(payload);
  enterCollapsed();
  pulseOrb();
}

function handleHide() {
  state.epoch += 1;
  clearHideTimer();
  state.text = "";
  state.source = "";
  state.busy = false;
  state.opening = false;
  state.pendingError = "";
  renderPayload();
  enterCollapsed();
}

async function onOrbClick() {
  if (state.expanded || state.opening) return;
  if (!tauriApi()) {
    enterBrowserMode();
    return;
  }

  state.opening = true;
  try {
    const epoch = state.epoch;
    if (!state.text.trim()) {
      state.pendingError = await pullPayload(epoch);
      if (epoch !== state.epoch) return;
    }

    try {
      await call("selection_expand", { expanded: true });
    } catch (err) {
      enterExpanded();
      setStatus("展开失败：" + messageOf(err), "err");
      return;
    }

    enterExpanded();

    if (state.pendingError) {
      setStatus(state.pendingError, "err");
      state.pendingError = "";
    } else if (!state.text.trim()) {
      setStatus("未获取到选中文字，请重新划词", "warn");
    }
  } finally {
    state.opening = false;
  }
}

async function hideAfterSave(epoch) {
  let failure = "";
  try {
    await call("selection_hide");
  } catch (err) {
    failure = "已存为日记 ✓（未能自动关闭：" + messageOf(err) + "）";
  }

  if (epoch !== state.epoch) return;

  if (failure) {
    state.busy = false;
    updateButtons();
    setStatus(failure, "warn");
    return;
  }

  enterCollapsed();
}

async function onJournal() {
  if (state.busy) return;
  const text = state.text;
  if (!text.trim()) {
    setStatus("没有可保存的文字", "warn");
    return;
  }

  const epoch = state.epoch;
  state.busy = true;
  updateButtons();
  setStatus("", "");

  try {
    const result = await call("selection_journal", { text });
    if (epoch !== state.epoch) return;
    if (!result || result.ok !== true) {
      const message =
        result && typeof result.message === "string" && result.message.trim()
          ? result.message.trim()
          : "保存失败，请重试";
      throw new Error(message);
    }

    const journal = $("btn-journal");
    if (journal) {
      journal.textContent = "已存为日记 ✓";
      journal.disabled = true;
    }
    setStatus("已存为日记 ✓", "ok");

    state.hideTimer = window.setTimeout(() => {
      state.hideTimer = 0;
      if (epoch !== state.epoch) return;
      void hideAfterSave(epoch);
    }, 1100);
  } catch (err) {
    if (epoch !== state.epoch) return;
    state.busy = false;
    updateButtons();
    setStatus(messageOf(err), "err");
  }
}

async function onEdit() {
  if (!state.text.trim()) {
    setStatus("没有可编辑的文字", "warn");
    return;
  }
  if (state.opening) return;

  const epoch = state.epoch;
  state.opening = true;
  try {
    await call("selection_open_main", { text: state.text });
  } catch (err) {
    if (epoch === state.epoch) {
      setStatus("打开主窗失败：" + messageOf(err), "err");
    }
  } finally {
    state.opening = false;
  }
}

async function onClose() {
  if (state.opening) return;
  if (!tauriApi()) return;

  state.opening = true;
  clearHideTimer();

  let failure = "";
  try {
    await call("selection_hide");
  } catch (err) {
    failure = "关闭失败：" + messageOf(err);
  } finally {
    state.opening = false;
  }

  if (failure) {
    setStatus(failure, "err");
    return;
  }

  enterCollapsed();
}

function onKeydown(event) {
  if (event.key !== "Escape") return;
  if (!state.expanded) return;
  if (!tauriApi()) return;
  event.preventDefault();
  void onClose();
}

function enterBrowserMode() {
  state.text = "";
  state.source = "";
  document.body.classList.add("no-tauri");
  enterExpanded();

  const journal = $("btn-journal");
  const edit = $("btn-edit");
  const closeBtn = $("btn-close");
  if (journal) journal.disabled = true;
  if (edit) edit.disabled = true;
  if (closeBtn) closeBtn.disabled = true;

  setStatus("请在 memTools 中打开", "warn");
}

async function init() {
  const orb = $("orb");
  const journal = $("btn-journal");
  const edit = $("btn-edit");
  const closeBtn = $("btn-close");

  if (orb) orb.addEventListener("click", () => void onOrbClick());
  if (journal) journal.addEventListener("click", () => void onJournal());
  if (edit) edit.addEventListener("click", () => void onEdit());
  if (closeBtn) closeBtn.addEventListener("click", () => void onClose());
  document.addEventListener("keydown", onKeydown);

  if (!tauriApi()) {
    enterBrowserMode();
    return;
  }

  /* 先注册事件，再主动拉一次，防止事件早于监听 */
  await listenEvent("selection://show", handleShow);
  await listenEvent("selection://hide", handleHide);

  const epoch = state.epoch;
  try {
    const payload = await call("selection_payload");
    if (epoch === state.epoch && payload) applyPayload(payload);
  } catch (err) {
    if (epoch === state.epoch) {
      state.pendingError = "读取选中文字失败：" + messageOf(err);
    }
  }

  updateButtons();
}

function boot() {
  void init().catch(() => {
    try {
      enterExpanded();
      setStatus("初始化失败，请在 memTools 中打开", "err");
    } catch (_) {
      /* 保持静默，避免控制台未捕获异常 */
    }
  });
}

if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", boot, { once: true });
} else {
  boot();
}
