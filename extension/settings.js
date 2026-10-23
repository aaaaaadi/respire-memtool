// 本机安装向导生成的私有 pairing.json 只在加载扩展时导入；公开源码不含令牌。
async function getSettings({ refresh = false } = {}) {
  const stored = await chrome.storage.local.get(["port", "token"]);
  let paired = null;
  if (refresh || !stored.token) {
    try {
      const response = await fetch(chrome.runtime.getURL("pairing.json"), { cache: "no-store" });
      if (response.ok) paired = await response.json();
    } catch (_) { /* 未配对的公开扩展仍支持手动设置。 */ }
  }
  if (paired && Number.isInteger(paired.port) && paired.port >= 1 && paired.port <= 65535 && typeof paired.token === "string" && paired.token.trim()) {
    const settings = { port: paired.port, token: paired.token.trim() };
    await chrome.storage.local.set(settings);
    return settings;
  }
  return {
    port: Number(stored.port) || 17650,
    token: typeof stored.token === "string" ? stored.token.trim() : "",
  };
}
