// 全部有效记忆的只读浏览：分类按真实祖先匹配，不把词库名当成 Runtime 节点 id。
export function selectMemories(items, { query = "", category = null, limit = 0 } = {}) {
  const index = new Map(items.map(entry => [entry.id, entry]));
  const terms = query.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
  const selected = items.filter(entry => {
    if (category) {
      const seen = new Set();
      let current = entry;
      let found = false;
      while (current && !seen.has(current.id)) {
        seen.add(current.id);
        if (current.id === category || current.title === category) { found = true; break; }
        current = index.get(current.parent_id);
      }
      if (!found) return false;
    }
    const text = [entry.id, entry.title, entry.content].filter(Boolean).join("\n").toLocaleLowerCase();
    return terms.every(term => text.includes(term));
  });
  return limit > 0 ? selected.slice(0, limit) : selected;
}

// 仅补足命中项的祖先；不混入未命中的兄弟，也不把路径节点计作搜索结果。
export function memoryForest(items, matches) {
  const index = new Map(items.map(entry => [entry.id, entry]));
  for (const entry of matches) index.set(entry.id, { ...index.get(entry.id), ...entry });
  const hits = new Set(matches.map(entry => entry.id));
  const included = new Set();
  for (const hit of matches) {
    let entry = index.get(hit.id);
    const seen = new Set();
    while (entry && !seen.has(entry.id)) {
      seen.add(entry.id);
      included.add(entry.id);
      entry = index.get(entry.parent_id);
    }
  }
  const nodes = new Map([...index].filter(([id]) => included.has(id)).map(([id, entry]) => [id, { entry, matched: hits.has(id), children: [] }]));
  const parents = new Map();
  const roots = [];
  for (const [id, node] of nodes) {
    let parent = nodes.has(node.entry.parent_id) ? node.entry.parent_id : null;
    const seen = new Set([id]);
    let ancestor = parent;
    while (ancestor) {
      if (seen.has(ancestor)) { parent = null; break; }
      seen.add(ancestor);
      ancestor = parents.get(ancestor);
    }
    parents.set(id, parent);
    if (parent) nodes.get(parent).children.push(node);
    else roots.push(node);
  }
  return roots;
}
