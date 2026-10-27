// 本机片段位置使用 JavaScript UTF-16 偏移，仅针对当前正文快照；不是 Runtime 的持久片段 ID。
export function isDiary(entry) {
  if (entry.importance) return entry.importance === "trivial";
  return /^活动轨迹\s+\d{4}-\d{2}-\d{2}/u.test(entry.title || "");
}
export function evidenceLabel(entry) {
  if (entry.superseded_by) return "历史 · 已被取代";
  return isDiary(entry) ? "日记 · 参考" : "参考 · 待核实";
}
export function queryPattern(query) {
  const terms = String(query || "").trim().split(/\s+/u).filter(Boolean);
  if (!terms.length) return null;
  return new RegExp(terms.map(term => term.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")).join("|"), "giu");
}
// 最多三个命中窗口，每个 360 UTF-16 单元；全文检索不截断，返回预览才截断。
export function matchingFragments(body, query) {
  body = String(body || "");
  const pattern = queryPattern(query);
  if (!pattern) return [];
  const boundaries = [...body.matchAll(/^(?:【|\[)(\d{2}:\d{2})(?:】|\])/gm)]
    .map(match => ({start: match.index, time: match[1]}));
  if (!boundaries.length || boundaries[0].start !== 0) boundaries.unshift({start:0, time:""});
  const result = [];
  let section = 0;
  let match;
  while (result.length < 3 && (match = pattern.exec(body))) {
    while (section + 1 < boundaries.length && boundaries[section + 1].start <= match.index) section++;
    const recordStart = boundaries[section].start;
    const recordEnd = boundaries[section + 1]?.start ?? body.length;
    let start = Math.max(recordStart, match.index - 90);
    let end = Math.min(recordEnd, start + 360);
    // 不把代理对切成孤立半字符，偏移仍准确指向原文。
    if (start > 0 && /[\uDC00-\uDFFF]/.test(body[start])) start++;
    if (end < body.length && /[\uDC00-\uDFFF]/.test(body[end])) end--;
    result.push({start, end, text:body.slice(start, end), time:boundaries[section].time});
    pattern.lastIndex = Math.max(end, pattern.lastIndex);
  }
  return result;
}
