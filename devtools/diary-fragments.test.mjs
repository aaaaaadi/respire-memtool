import test from "node:test";
import assert from "node:assert/strict";
import * as fragments from "../src/diary-fragments.js";
test("正文末尾命中不受预览长度限制，位置指向原文且片段有界", () => {
  assert.equal(typeof fragments.matchingFragments, "function");
  const body = "【08:00】\n" + "无关😀".repeat(30000) + "\n【16:52】\n物联网接入：角色权限。";
  const hits = fragments.matchingFragments(body, "角色权限");
  assert.equal(hits.length, 1);
  assert.ok(hits[0].text.includes("角色权限"));
  assert.ok(hits[0].text.length <= 360);
  assert.equal(body.slice(hits[0].start, hits[0].end), hits[0].text);
  assert.equal(hits[0].time, "16:52");
});
test("多个主题分别定位，不将关键词分布在不同记录误判为同一结论", () => {
  const hits = fragments.matchingFragments("【08:00】设备接入\n【09:00】角色权限", "设备 权限");
  assert.equal(hits.length, 2);
  assert.equal(hits[0].time, "08:00");
  assert.equal(hits[1].time, "09:00");
});
test("重复命中限制预览数，不切断 emoji，空查询不返回整个日记", () => {
  const body = "😀港珠澳".repeat(10000);
  const hits = fragments.matchingFragments(body, "港珠澳");
  assert.ok(hits.length <= 3);
  assert.ok(hits.every(h => h.text.isWellFormed() && h.text.length <= 360));
  assert.deepEqual(fragments.matchingFragments(body, ""), []);
});
test("资料标识不把重要性或相似度当成可信度", () => {
  assert.equal(fragments.evidenceLabel({importance:"important",score:1}), "参考 · 待核实");
  assert.equal(fragments.evidenceLabel({title:"活动轨迹 2026-10-26",importance:"trivial"}), "日记 · 参考");
  assert.equal(fragments.evidenceLabel({superseded_by:"new"}), "历史 · 已被取代");
});

test("日记使用实际重要性而非标题判定", () => {
  assert.equal(fragments.isDiary({importance:"trivial",title:"改名日记"}), true);
  assert.equal(fragments.isDiary({importance:"important",title:"活动轨迹 2026-10-26"}), false);
});
