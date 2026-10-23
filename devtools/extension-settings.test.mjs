import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import vm from "node:vm";

function extension(settings, pairing) {
  const store = { ...settings };
  const event = { addListener() {} };
  const installed = [];
  const context = vm.createContext({
    chrome: {
      contextMenus: { removeAll(cb) { cb(); }, create() {}, onClicked: event },
      runtime: { onInstalled: { addListener(fn) { installed.push(fn); } }, onStartup: event, onMessage: event, getURL: p => "chrome-extension://test/" + p },
      action: { onClicked: event },
      alarms: { get(_name, cb) { cb({}); }, onAlarm: event },
      storage: { local: { async get() { return { ...store }; }, async set(v) { Object.assign(store, v); } } },
    },
    async fetch(url) {
      assert.equal(url, "chrome-extension://test/pairing.json");
      return { ok: true, async json() { return pairing; } };
    },
  });
  context.importScripts = file => vm.runInContext(fs.readFileSync(new URL("../extension/" + file, import.meta.url), "utf8"), context);
  vm.runInContext(fs.readFileSync(new URL("../extension/background.js", import.meta.url), "utf8"), context);
  return { async install() { for (const listener of installed) await listener({ reason: "update" }); }, async get(refresh = false) { return JSON.parse(JSON.stringify(await vm.runInContext("getSettings({ refresh: " + refresh + " })", context))); }, store };
}

test("newly loaded private extension pairs without copying a token", async () => {
  const ext = extension({}, { port: 18765, token: "test-only-pairing-fixture" });
  assert.deepEqual(await ext.get(), { port: 18765, token: "test-only-pairing-fixture" });
  assert.equal(ext.store.token, "test-only-pairing-fixture");
});

test("public extension with no bundled token keeps manual settings", async () => {
  const ext = extension({ port: 17650, token: "test-only-manual-fixture" }, { port: 17650, token: "" });
  assert.deepEqual(await ext.get(), { port: 17650, token: "test-only-manual-fixture" });
});

test("reloading a freshly prepared extension replaces an obsolete pairing", async () => {
  const ext = extension({ port: 17650, token: "old-fixture" }, { port: 18765, token: "new-fixture" });
  await ext.install();
  assert.deepEqual(await ext.get(), { port: 18765, token: "new-fixture" });
});

test("invalid packaged port cannot replace stored connection settings", async () => {
  const ext = extension({ port: 17650, token: "manual-fixture" }, { port: 70000, token: "bad-fixture" });
  assert.deepEqual(await ext.get(true), { port: 17650, token: "manual-fixture" });
});
