/** 从屏幕坐标选择节点；重叠实体优先前景，扩展点击区优先最近中心。 */
export function pickProjectedNode(points, pointer, minimumRadius = 12) {
  let best = null;
  for (const point of points) {
    if (!(point.z > 0) || !Number.isFinite(point.x + point.y + point.z)) continue;
    const distance = Math.hypot(point.x - pointer.x, point.y - pointer.y);
    const radius = Math.max(0, point.radius || 0);
    if (distance > Math.max(minimumRadius, radius)) continue;
    const inside = distance <= radius;
    if (!best || (inside && !best.inside) || (inside === best.inside &&
        (inside ? point.z < best.point.z : distance < best.distance))) {
      best = { point, distance, inside };
    }
  }
  return best?.point.node ?? null;
}

/** 只允许同一主键手势的小幅移动；曾经拖远再回原位也不视为点击。 */
export function isGraphClick(start, end) {
  return !!start && start.button === 0 && end.button === 0 &&
    start.pointerId === end.pointerId &&
    Math.max(start.maxDistance || 0, Math.hypot(start.x - end.x, start.y - end.y)) <= 6;
}

/** 在已加载图谱中匹配全部关键词；不发 Runtime 请求、不解释 HTML。 */
export function searchGraphNodes(nodes, query) {
  const terms = String(query || '').trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
  if (!terms.length) return [];
  return nodes.filter(node => {
    // 标题/name、稳定ID/id、分类/category、标签/tags 与正文预览/content_head 均来自图谱快照；缺省按空串检索。
    const haystack = [node.name, node.id, node.category, ...(node.tags || []), node.content_head]
      .filter(Boolean).join(' ').toLocaleLowerCase();
    return terms.every(term => haystack.includes(term));
  });
}
