// memTools 扩展选项：保存端口/令牌，测试 /health。

const $ = (s) => document.querySelector(s);
const statusEl = $("#status");

function setStatus(text, kind) {
  statusEl.textContent = text;
  statusEl.className = kind || "";
}

async function load() {
  const got = await getSettings();
  if (got.port) $("#port").value = String(got.port);
  if (typeof got.token === "string") $("#token").value = got.token;
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

$("#save").addEventListener("click", save);
$("#test").addEventListener("click", test);
load();
