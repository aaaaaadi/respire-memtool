//! 图谱数据层：把 rsrs 本地记忆库映射成 3D 节点/边，并提供实时增量。
//!
//! 设计要点（与 mind3D demo6 的 bridge 等价，但内嵌在 mem-tools 里）：
//!   - 快照：只读 SQLite `memories` + `core_artifacts` + `recall_pairs`，
//!           从 `core-index` 读 m3 向量并投影到 3D（冻结 PCA 基，布局不重排）。
//!   - 实时：轮询 SQLite `entry_audit`（create/update/delete/restore/purge）
//!           与 `query_log`（recall → focus 点亮）。任何宿主（memTools / opencode /
//!           MCP / CLI / 云端同步）写同一个库都会被看到，**不需要 plugins.json hook**。
//!   - 安全：SQLite 只读打开；core-index 只读；不启动/停止 rsrs 服务。
//!
//! 这样就不需要额外运行 mind3D 的 Node bridge（127.0.0.1:8791）。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

use crate::graph_vectors::{
    chrono_like_now, load_or_build_basis, load_vectors, project, project_all, read_vector_for_id,
    Basis,
};

// ---------------------------------------------------------------------------
// 对外类型
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, Default)]
pub struct GraphCursor {
    pub seq: i64,
    pub qid: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphNode {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub importance: String,
    pub tags: Vec<String>,
    pub parent_id: String,
    pub recall_count: u32,
    pub content_head: String,
    pub created_at: String,
    pub updated_at: String,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    /// 沿 parent_id 上溯得到的根标题（日记类归入「日记」）
    pub category: String,
    /// 根索引（前端用它算色相；-1 = 无根，回退 kind 色）
    pub root_index: i32,
    /// 根的直接子节点 id（前端用它算同根色相微差）
    pub branch_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphLink {
    pub id: String,
    pub source: String,
    pub target: String,
    #[serde(rename = "type")]
    pub link_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphRoot {
    pub index: i32,
    pub title: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphStats {
    pub nodes: usize,
    pub links: usize,
    pub recall_max: u32,
    pub vectors: usize,
    pub missing_vectors: usize,
    pub basis_cached: bool,
    pub ms: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphSnapshot {
    pub nodes: Vec<GraphNode>,
    pub links: Vec<GraphLink>,
    pub roots: Vec<GraphRoot>,
    pub cursor: GraphCursor,
    pub stats: GraphStats,
    pub generated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphEvent {
    pub op: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<GraphNode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<GraphLink>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl GraphEvent {
    fn node_add(node: GraphNode) -> Self {
        Self {
            op: "node.add".into(),
            data: Some(node),
            id: None,
            ids: None,
            parent_id: None,
            link: None,
            query: None,
            note: None,
        }
    }

    fn node_update(node: GraphNode) -> Self {
        Self {
            op: "node.update".into(),
            data: Some(node),
            id: None,
            ids: None,
            parent_id: None,
            link: None,
            query: None,
            note: None,
        }
    }

    fn node_remove(id: impl Into<String>) -> Self {
        Self {
            op: "node.remove".into(),
            data: None,
            id: Some(id.into()),
            ids: None,
            parent_id: None,
            link: None,
            query: None,
            note: None,
        }
    }

    fn link_add(link: GraphLink) -> Self {
        Self {
            op: "link.add".into(),
            data: None,
            id: None,
            ids: None,
            parent_id: None,
            link: Some(link),
            query: None,
            note: None,
        }
    }

    fn relink(id: impl Into<String>, parent_id: impl Into<String>) -> Self {
        Self {
            op: "relink".into(),
            data: None,
            id: Some(id.into()),
            ids: None,
            parent_id: Some(parent_id.into()),
            link: None,
            query: None,
            note: None,
        }
    }

    fn focus(ids: Vec<String>, query: String) -> Self {
        Self {
            op: "focus".into(),
            data: None,
            id: None,
            ids: Some(ids),
            parent_id: None,
            link: None,
            query: Some(query),
            note: None,
        }
    }

}

#[derive(Debug, Clone, Serialize)]
pub struct GraphEventsOut {
    pub ops: Vec<GraphEvent>,
    pub cursor: GraphCursor,
    pub reset: bool,
    pub has_more: bool,
}

// ---------------------------------------------------------------------------
// 内部模型
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct EntryMeta {
    id: String,
    title: String,
    content_head: String,
    kind: String,
    importance: String,
    tags: Vec<String>,
    parent_id: String,
    created_at: String,
    updated_at: String,
    recall_count: u32,
    deleted: bool,
}

struct Inner {
    data_dir: PathBuf,
    db_path: PathBuf,
    core_dir: PathBuf,
    basis: Basis,
    coords: HashMap<String, [f32; 3]>,
    entries: HashMap<String, EntryMeta>,
    artifacts: HashMap<String, Vec<String>>,
    root_index: HashMap<String, i32>,
    /// 元数据增量游标：用于捕捉 kind/importance/tags 等不触发 entry_audit 的变更
    last_updated_at: String,
    last_updated_id: String,
}

/// 图谱服务：由 Tauri 以 `Arc<GraphService>` 管理。
pub struct GraphService {
    cache_dir: PathBuf,
    inner: Mutex<Option<Inner>>,
}

impl GraphService {
    pub fn new(cache_dir: PathBuf) -> Self {
        Self {
            cache_dir,
            inner: Mutex::new(None),
        }
    }

    /// 全量快照。`force_layout=true` 会无视 basis 缓存重新计算 PCA（布局重排）。
    pub fn snapshot(&self, data_dir: &Path) -> Result<GraphSnapshot, String> {
        self.snapshot_with(data_dir, false)
    }

    pub fn rebuild_layout(&self) -> Result<GraphSnapshot, String> {
        let data_dir = {
            let guard = self
                .inner
                .lock()
                .map_err(|_| "图谱状态锁失效".to_string())?;
            match guard.as_ref() {
                Some(inner) => inner.data_dir.clone(),
                None => return Err("图谱尚未加载，请先打开图谱视图".into()),
            }
        };
        self.snapshot_with(&data_dir, true)
    }

    fn snapshot_with(&self, data_dir: &Path, force_layout: bool) -> Result<GraphSnapshot, String> {
        let t0 = Instant::now();
        let data_dir = data_dir.to_path_buf();
        let db_path = data_dir.join("onememory.db");
        if !db_path.exists() {
            return Err(format!(
                "未找到 rsrs 数据库：{}\n请确认 rsrs runtime 已启动，或该账户的数据目录正确。",
                db_path.display()
            ));
        }
        let conn = open_db(&db_path)?;
        let entries = load_entries(&conn)?;
        let core_dir = data_dir.join("core-index");
        let (vectors, artifacts, missing_vectors, vector_note) =
            load_vectors(&conn, &core_dir, &self.cache_dir);
        let (basis, basis_cached) = load_or_build_basis(&self.cache_dir, &vectors, force_layout);

        let mut coords = project_all(&vectors, &basis);
        fill_missing_coords(&entries, &mut coords);

        let (roots, root_index) = build_roots(&entries);
        let pairs = read_recall_pairs(&conn)?;
        let (nodes, links, recall_max) = build_graph(&entries, &coords, &pairs, &root_index);
        let (seq, qid) = read_cursors(&conn)?;

        let mut last_updated_at = String::new();
        let mut last_updated_id = String::new();
        for e in entries.values() {
            if e.updated_at > last_updated_at
                || (e.updated_at == last_updated_at && e.id > last_updated_id)
            {
                last_updated_at = e.updated_at.clone();
                last_updated_id = e.id.clone();
            }
        }
        let inner = Inner {
            data_dir,
            db_path,
            core_dir,
            basis,
            coords,
            entries,
            artifacts,
            root_index,
            last_updated_at,
            last_updated_id,
        };
        *self
            .inner
            .lock()
            .map_err(|_| "图谱状态锁失效".to_string())? = Some(inner);

        let ms = t0.elapsed().as_millis();
        let node_count = nodes.len();
        let link_count = links.len();
        Ok(GraphSnapshot {
            nodes,
            links,
            roots,
            cursor: GraphCursor { seq, qid },
            stats: GraphStats {
                nodes: node_count,
                links: link_count,
                recall_max,
                vectors: vectors.len(),
                missing_vectors,
                basis_cached,
                ms,
                note: vector_note,
            },
            generated_at: chrono_like_now(),
        })
    }

    /// 从上次游标起取增量事件：本地/其它宿主的写入、云端同步、recall 点亮。
    pub fn events_since(&self, seq: i64, qid: i64) -> Result<GraphEventsOut, String> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| "图谱状态锁失效".to_string())?;
        let inner = guard
            .as_mut()
            .ok_or_else(|| "图谱尚未加载，请先打开图谱视图".to_string())?;

        let conn = open_db(&inner.db_path)?;
        let (max_seq, max_qid) = read_cursors(&conn)?;

        // 数据库被恢复/重建后 seq 可能回退：要求前端重新拉全量快照
        let reset = max_seq < seq || max_qid < qid;
        if reset {
            return Ok(GraphEventsOut {
                ops: Vec::new(),
                cursor: GraphCursor {
                    seq: max_seq,
                    qid: max_qid,
                },
                reset: true,
                has_more: false,
            });
        }

        let mut cur = GraphCursor { seq, qid };
        let mut ops: Vec<GraphEvent> = Vec::new();
        let mut touched: HashSet<String> = HashSet::new();

        // ---- entry_audit：创建/更新/删除/恢复/清除 ----
        let mut audit_rows: Vec<(i64, String, String)> = Vec::new();
        {
            let mut stmt = conn
                .prepare(
                    "SELECT seq, entry_id, action FROM entry_audit \
                     WHERE seq > ?1 ORDER BY seq ASC LIMIT 201",
                )
                .map_err(|e| format!("查询 entry_audit 失败: {e}"))?;
            let mapped = stmt
                .query_map([cur.seq], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })
                .map_err(|e| format!("读取 entry_audit 失败: {e}"))?;
            for row in mapped {
                audit_rows.push(row.map_err(|e| format!("解析 entry_audit 行失败: {e}"))?);
            }
        }
        let mut has_more = audit_rows.len() > 200;
        if has_more {
            audit_rows.truncate(200);
        }

        for (seq, id, action) in audit_rows {
            cur.seq = seq;
            let entry = match load_entry(&conn, &id)? {
                Some(e) => e,
                None => {
                    // purge：行已不存在，仍要通知前端删节点
                    if action == "purge" || action == "delete" {
                        inner.entries.remove(&id);
                        inner.coords.remove(&id);
                        ops.push(GraphEvent::node_remove(&id));
                        touched.insert(id.clone());
                    }
                    continue;
                }
            };
            match action.as_str() {
                "create" | "restore" => {
                    touched.insert(id.clone());
                    if entry.deleted {
                        inner.entries.remove(&id);
                        inner.coords.remove(&id);
                        ops.push(GraphEvent::node_remove(&id));
                        continue;
                    }
                    inner.entries.insert(id.clone(), entry.clone());
                    let coord = ensure_coord(inner, &conn, &entry);
                    let node = build_node(&entry, coord, &inner.entries, &inner.root_index);
                    ops.push(GraphEvent::node_add(node));
                    let pid = entry.parent_id.clone();
                    if !pid.is_empty() && inner.entries.contains_key(&pid) {
                        ops.push(GraphEvent::link_add(parent_link(&pid, &id)));
                    }
                }
                "update" => {
                    touched.insert(id.clone());
                    if entry.deleted {
                        inner.entries.remove(&id);
                        inner.coords.remove(&id);
                        ops.push(GraphEvent::node_remove(&id));
                        continue;
                    }
                    inner.entries.insert(id.clone(), entry.clone());
                    let coord = ensure_coord(inner, &conn, &entry);
                    let node = build_node(&entry, coord, &inner.entries, &inner.root_index);
                    ops.push(GraphEvent::node_update(node));
                    ops.push(GraphEvent::relink(&id, &entry.parent_id));
                }
                "delete" | "purge" => {
                    touched.insert(id.clone());
                    inner.entries.remove(&id);
                    inner.coords.remove(&id);
                    ops.push(GraphEvent::node_remove(&id));
                }
                _ => {}
            }
        }

        // ---- memories.updated_at：补齐不触发 entry_audit 的元数据变更 ----
        // entry_audit 只在 title/content_head/parent_id 变化时触发；kind/importance/tags
        // 的 update 不会进审计流，这里按 (updated_at,id) 键集游标补一轮。
        let mut meta_rows: Vec<(String, String)> = Vec::new();
        {
            let mut stmt = conn
                .prepare(
                    "SELECT updated_at, id FROM memories \
                     WHERE updated_at > ?1 OR (updated_at = ?1 AND id > ?2) \
                     ORDER BY updated_at ASC, id ASC LIMIT 201",
                )
                .map_err(|e| format!("查询 memories 元数据失败: {e}"))?;
            let mapped = stmt
                .query_map([&inner.last_updated_at, &inner.last_updated_id], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })
                .map_err(|e| format!("读取 memories 元数据失败: {e}"))?;
            for row in mapped {
                meta_rows.push(row.map_err(|e| format!("解析 memory 元数据失败: {e}"))?);
            }
        }
        if meta_rows.len() > 200 {
            has_more = true;
            meta_rows.truncate(200);
        }
        for (updated_at, id) in meta_rows {
            inner.last_updated_at = updated_at;
            inner.last_updated_id = id.clone();
            if touched.contains(&id) {
                continue;
            }
            let Some(entry) = load_entry(&conn, &id)? else {
                continue;
            };
            if entry.deleted {
                if inner.entries.remove(&id).is_some() {
                    inner.coords.remove(&id);
                    ops.push(GraphEvent::node_remove(&id));
                }
                continue;
            }
            let is_new = !inner.entries.contains_key(&id);
            let changed = match inner.entries.get(&id) {
                None => true,
                Some(old) => metadata_changed(old, &entry),
            };
            if !changed {
                continue;
            }
            inner.entries.insert(id.clone(), entry.clone());
            let coord = ensure_coord(inner, &conn, &entry);
            let node = build_node(&entry, coord, &inner.entries, &inner.root_index);
            if is_new {
                ops.push(GraphEvent::node_add(node));
                let pid = entry.parent_id.clone();
                if !pid.is_empty() && inner.entries.contains_key(&pid) {
                    ops.push(GraphEvent::link_add(parent_link(&pid, &id)));
                }
            } else {
                ops.push(GraphEvent::node_update(node));
                ops.push(GraphEvent::relink(&id, &entry.parent_id));
            }
        }

        // ---- query_log：recall → focus 点亮 ----
        let mut qrows: Vec<(i64, String, String, String)> = Vec::new();
        {
            let mut stmt = conn
                .prepare(
                    "SELECT id, query, candidates, adopted FROM query_log \
                     WHERE id > ?1 ORDER BY id ASC LIMIT 51",
                )
                .map_err(|e| format!("查询 query_log 失败: {e}"))?;
            let mapped = stmt
                .query_map([cur.qid], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                })
                .map_err(|e| format!("读取 query_log 失败: {e}"))?;
            for row in mapped {
                qrows.push(row.map_err(|e| format!("解析 query_log 行失败: {e}"))?);
            }
        }
        if qrows.len() > 50 {
            has_more = true;
            qrows.truncate(50);
        }
        for (id, query, candidates, adopted) in qrows {
            cur.qid = id;
            let mut ids = parse_id_list(&adopted);
            if ids.is_empty() {
                ids = parse_id_list(&candidates);
            }
            ids.retain(|x| inner.entries.contains_key(x));
            ids.truncate(16);
            if !ids.is_empty() {
                ops.push(GraphEvent::focus(ids, query));
            }
        }

        Ok(GraphEventsOut {
            ops,
            cursor: cur,
            reset: false,
            has_more,
        })
    }
}

// ---------------------------------------------------------------------------
// 文件/DB 辅助
// ---------------------------------------------------------------------------

fn open_db(path: &Path) -> Result<Connection, String> {
    match Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY) {
        Ok(conn) => {
            let _ = conn.busy_timeout(Duration::from_millis(3000));
            Ok(conn)
        }
        Err(e) => {
            // runtime 停止且 WAL 缺 shm 时，只读连接可能打不开；退化到 immutable
            // （可能看不到最新 WAL 内容，但至少能打开旧快照）。
            let uri = format!(
                "file:{}?mode=ro&immutable=1",
                path.to_string_lossy().replace('\\', "/")
            );
            let conn = Connection::open_with_flags(
                &uri,
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
            )
            .map_err(|e2| {
                format!(
                    "打开 rsrs 数据库失败（只读）：{}: {e}；immutable 回退也失败：{e2}",
                    path.display()
                )
            })?;
            let _ = conn.busy_timeout(Duration::from_millis(3000));
            Ok(conn)
        }
    }
}

fn split_tags(s: &str) -> Vec<String> {
    s.split(',')
        .map(|x| x.trim())
        .filter(|x| !x.is_empty())
        .map(|x| x.to_string())
        .collect()
}

fn load_entries(conn: &Connection) -> Result<HashMap<String, EntryMeta>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, title, content_head, kind, importance, tags, parent_id, \
             created_at, updated_at, recall_count FROM memories WHERE deleted = 0",
        )
        .map_err(|e| format!("查询 memories 失败: {e}"))?;
    let rows = stmt
        .query_map([], |r| {
            Ok(EntryMeta {
                id: r.get(0)?,
                title: r.get(1)?,
                content_head: r.get(2)?,
                kind: r.get(3)?,
                importance: r.get(4)?,
                tags: split_tags(&r.get::<_, String>(5)?),
                parent_id: r.get(6)?,
                created_at: r.get(7)?,
                updated_at: r.get(8)?,
                recall_count: r.get::<_, i64>(9).unwrap_or(0).max(0) as u32,
                deleted: false,
            })
        })
        .map_err(|e| format!("读取 memories 失败: {e}"))?;
    let mut out = HashMap::new();
    for row in rows {
        let e = row.map_err(|e| format!("解析 memory 行失败: {e}"))?;
        out.insert(e.id.clone(), e);
    }
    Ok(out)
}

fn load_entry(conn: &Connection, id: &str) -> Result<Option<EntryMeta>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, title, content_head, kind, importance, tags, parent_id, \
             created_at, updated_at, recall_count, deleted FROM memories WHERE id = ?1",
        )
        .map_err(|e| format!("查询单条 memory 失败: {e}"))?;
    let mut rows = stmt
        .query([id])
        .map_err(|e| format!("读取单条 memory 失败: {e}"))?;
    let Some(row) = rows.next().map_err(|e| format!("遍历 memory 行失败: {e}"))? else {
        return Ok(None);
    };
    let deleted: i64 = row.get(10).unwrap_or(0);
    Ok(Some(EntryMeta {
        id: row.get(0).map_err(|e| e.to_string())?,
        title: row.get(1).unwrap_or_default(),
        content_head: row.get(2).unwrap_or_default(),
        kind: row.get(3).unwrap_or_default(),
        importance: row.get(4).unwrap_or_default(),
        tags: split_tags(&row.get::<_, String>(5).unwrap_or_default()),
        parent_id: row.get(6).unwrap_or_default(),
        created_at: row.get(7).unwrap_or_default(),
        updated_at: row.get(8).unwrap_or_default(),
        recall_count: row.get::<_, i64>(9).unwrap_or(0).max(0) as u32,
        deleted: deleted != 0,
    }))
}

fn read_recall_pairs(conn: &Connection) -> Result<Vec<(String, String, u32)>, String> {
    let mut stmt = conn
        .prepare("SELECT a, b, n FROM recall_pairs")
        .map_err(|e| format!("查询 recall_pairs 失败: {e}"))?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2).unwrap_or(1).max(1) as u32,
            ))
        })
        .map_err(|e| format!("读取 recall_pairs 失败: {e}"))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| format!("解析 recall_pair 失败: {e}"))?);
    }
    Ok(out)
}

fn read_cursors(conn: &Connection) -> Result<(i64, i64), String> {
    let seq: i64 = conn
        .query_row("SELECT COALESCE(MAX(seq), 0) FROM entry_audit", [], |r| {
            r.get(0)
        })
        .unwrap_or(0);
    let qid: i64 = conn
        .query_row("SELECT COALESCE(MAX(id), 0) FROM query_log", [], |r| r.get(0))
        .unwrap_or(0);
    Ok((seq, qid))
}

// ---------------------------------------------------------------------------
// 根/节点/边
// ---------------------------------------------------------------------------

fn build_roots(entries: &HashMap<String, EntryMeta>) -> (Vec<GraphRoot>, HashMap<String, i32>) {
    let mut roots = Vec::new();
    let mut map: HashMap<String, i32> = HashMap::new();
    let has_diary = entries
        .values()
        .any(|e| e.parent_id.is_empty() && e.title.starts_with("活动轨迹"));
    let mut named: Vec<(String, String)> = entries
        .values()
        .filter(|e| e.parent_id.is_empty() && !e.title.starts_with("活动轨迹"))
        .map(|e| (e.title.clone(), e.created_at.clone()))
        .collect();
    named.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
    named.dedup_by(|a, b| a.0 == b.0);

    let mut idx = 0i32;
    if has_diary {
        roots.push(GraphRoot {
            index: 0,
            title: "日记".into(),
        });
        map.insert("日记".into(), 0);
        idx = 1;
    }
    for (title, _) in named {
        if title.is_empty() || map.contains_key(&title) {
            continue;
        }
        roots.push(GraphRoot {
            index: idx,
            title: title.clone(),
        });
        map.insert(title, idx);
        idx += 1;
    }
    (roots, map)
}

fn resolve_root(
    meta: &EntryMeta,
    entries: &HashMap<String, EntryMeta>,
    root_index: &HashMap<String, i32>,
) -> (i32, String, String) {
    let mut chain: Vec<&EntryMeta> = Vec::new();
    let mut cur = meta;
    let mut seen: HashSet<String> = HashSet::new();
    loop {
        if !seen.insert(cur.id.clone()) {
            break;
        }
        chain.push(cur);
        if cur.parent_id.is_empty() {
            break;
        }
        match entries.get(&cur.parent_id) {
            Some(p) => cur = p,
            None => break,
        }
    }
    let top = match chain.last() {
        Some(t) => *t,
        None => return (-1, String::new(), String::new()),
    };
    let category = if top.title.starts_with("活动轨迹") {
        "日记".to_string()
    } else {
        top.title.clone()
    };
    let index = *root_index.get(&category).unwrap_or(&-1);
    let branch_id = if chain.len() >= 2 {
        chain[chain.len() - 2].id.clone()
    } else {
        top.id.clone()
    };
    (index, category, branch_id)
}

/// 只看图谱可见字段；仅 updated_at 变化（如日记追加正文）不算变化。
fn metadata_changed(old: &EntryMeta, new: &EntryMeta) -> bool {
    old.title != new.title
        || old.kind != new.kind
        || old.importance != new.importance
        || old.tags != new.tags
        || old.parent_id != new.parent_id
        || old.recall_count != new.recall_count
        || old.content_head != new.content_head
        || old.deleted != new.deleted
}

fn build_node(
    meta: &EntryMeta,
    coord: [f32; 3],
    entries: &HashMap<String, EntryMeta>,
    root_index: &HashMap<String, i32>,
) -> GraphNode {
    let (index, category, branch_id) = resolve_root(meta, entries, root_index);
    GraphNode {
        id: meta.id.clone(),
        name: meta.title.clone(),
        kind: if meta.kind.is_empty() {
            "context".into()
        } else {
            meta.kind.clone()
        },
        importance: if meta.importance.is_empty() {
            "normal".into()
        } else {
            meta.importance.clone()
        },
        tags: meta.tags.clone(),
        parent_id: meta.parent_id.clone(),
        recall_count: meta.recall_count,
        content_head: meta.content_head.clone(),
        created_at: meta.created_at.clone(),
        updated_at: meta.updated_at.clone(),
        x: coord[0] as f64,
        y: coord[1] as f64,
        z: coord[2] as f64,
        category,
        root_index: index,
        branch_id,
    }
}

fn parent_link(pid: &str, cid: &str) -> GraphLink {
    GraphLink {
        id: format!("p_{pid}_{cid}"),
        source: pid.to_string(),
        target: cid.to_string(),
        link_type: "parent".into(),
        n: None,
    }
}

fn build_graph(
    entries: &HashMap<String, EntryMeta>,
    coords: &HashMap<String, [f32; 3]>,
    pairs: &[(String, String, u32)],
    root_index: &HashMap<String, i32>,
) -> (Vec<GraphNode>, Vec<GraphLink>, u32) {
    let mut metas: Vec<&EntryMeta> = entries.values().collect();
    metas.sort_by(|a, b| {
        a.created_at
            .cmp(&b.created_at)
            .then_with(|| a.id.cmp(&b.id))
    });

    let mut nodes = Vec::with_capacity(metas.len());
    let mut recall_max = 0u32;
    for m in &metas {
        if m.recall_count > recall_max {
            recall_max = m.recall_count;
        }
        let coord = coords.get(&m.id).copied().unwrap_or([0.0, 0.0, 0.0]);
        nodes.push(build_node(m, coord, entries, root_index));
    }

    let mut links = Vec::new();
    let mut seen = HashSet::new();
    for m in &metas {
        if !m.parent_id.is_empty() && entries.contains_key(&m.parent_id) {
            let link = parent_link(&m.parent_id, &m.id);
            if seen.insert(link.id.clone()) {
                links.push(link);
            }
        }
    }
    for (a, b, n) in pairs {
        if a == b || !entries.contains_key(a) || !entries.contains_key(b) {
            continue;
        }
        let (s, t) = if a < b {
            (a.clone(), b.clone())
        } else {
            (b.clone(), a.clone())
        };
        let id = format!("c_{s}_{t}");
        if seen.insert(id.clone()) {
            links.push(GraphLink {
                id,
                source: s,
                target: t,
                link_type: "corecall".into(),
                n: Some(*n),
            });
        }
    }
    (nodes, links, recall_max)
}

// ---------------------------------------------------------------------------
// 坐标兜底 / 增量投影
// ---------------------------------------------------------------------------

fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x1000_0000_01b3);
    }
    h
}

fn fill_missing_coords(
    entries: &HashMap<String, EntryMeta>,
    coords: &mut HashMap<String, [f32; 3]>,
) {
    let missing: Vec<String> = entries
        .keys()
        .filter(|id| !coords.contains_key(*id))
        .cloned()
        .collect();
    for id in missing {
        let mut cur = entries.get(&id);
        let mut anchor: Option<[f32; 3]> = None;
        let mut depth = 0usize;
        let mut seen: HashSet<String> = HashSet::new();
        while let Some(m) = cur {
            if !seen.insert(m.id.clone()) {
                break;
            }
            if let Some(c) = coords.get(&m.id) {
                anchor = Some(*c);
                break;
            }
            depth += 1;
            cur = entries.get(&m.parent_id);
        }
        let h = fnv1a(&id);
        let angle = (h % 3600) as f32 / 3600.0 * std::f32::consts::TAU;
        let y = ((h >> 12) % 200) as f32 / 200.0 * 160.0 - 80.0;
        let c = if let Some(a) = anchor {
            let r = 16.0 + (depth.min(6) as f32) * 4.0;
            [
                a[0] + angle.cos() * r,
                a[1] + y * 0.35,
                a[2] + angle.sin() * r,
            ]
        } else {
            let r = 620.0;
            [angle.cos() * r, y, angle.sin() * r]
        };
        coords.insert(id, c);
    }
}

fn ensure_coord(inner: &mut Inner, conn: &Connection, meta: &EntryMeta) -> [f32; 3] {
    if let Some(c) = inner.coords.get(&meta.id) {
        return *c;
    }
    let v = read_vector_for_id(
        conn,
        &inner.core_dir,
        &meta.id,
        Some(&inner.artifacts),
    );
    let c = if let Some(v) = v {
        project(Some(&v), &inner.basis)
    } else {
        // 无向量：贴着父节点放，避免所有缺失条目堆在原点
        let mut anchor = None;
        let mut cur = inner.entries.get(&meta.parent_id);
        for _ in 0..8 {
            let Some(m) = cur else { break };
            if let Some(p) = inner.coords.get(&m.id) {
                anchor = Some(*p);
                break;
            }
            cur = inner.entries.get(&m.parent_id);
        }
        let h = fnv1a(&meta.id);
        let angle = (h % 3600) as f32 / 3600.0 * std::f32::consts::TAU;
        match anchor {
            Some(a) => [
                a[0] + angle.cos() * 18.0,
                a[1] + ((h >> 8) % 80) as f32 - 40.0,
                a[2] + angle.sin() * 18.0,
            ],
            None => [
                angle.cos() * 620.0,
                ((h >> 12) % 200) as f32 - 100.0,
                angle.sin() * 620.0,
            ],
        }
    };
    inner.coords.insert(meta.id.clone(), c);
    c
}

// ---------------------------------------------------------------------------
// recall 候选解析
// ---------------------------------------------------------------------------

fn parse_id_list(raw: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<Vec<serde_json::Value>>(raw) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for x in v {
        let id = match x {
            serde_json::Value::String(s) => s,
            serde_json::Value::Object(o) => o
                .get("id")
                .or_else(|| o.get("entry_id"))
                .or_else(|| o.get("memory_id"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_default(),
            _ => String::new(),
        };
        if looks_like_uuid(&id) && !out.iter().any(|x| x == &id) {
            out.push(id);
        }
    }
    out
}

fn looks_like_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 36 {
        return false;
    }
    for (i, c) in b.iter().enumerate() {
        let ok = match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_hexdigit(),
        };
        if !ok {
            return false;
        }
    }
    true
}

// ---------------------------------------------------------------------------
// 数据目录探测
// ---------------------------------------------------------------------------

/// 只有显式设置 RSRS_DATA_DIR 时才使用override目录；不做账号猜测。
pub fn data_dir_override() -> Option<PathBuf> {
    std::env::var("RSRS_DATA_DIR")
        .ok()
        .map(|d| d.trim().to_string())
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
}

/// 猜测 rsrs 数据目录（运行时不可达时兜底）：~/.rsrs/accounts 下最近修改的库
pub fn guess_data_dir() -> PathBuf {
    let base = dirs::home_dir()
        .unwrap_or_else(|| std::env::temp_dir())
        .join(".rsrs")
        .join("accounts");
    if !base.exists() {
        return std::env::temp_dir();
    }
    let mut latest: Option<(PathBuf, std::time::SystemTime)> = None;
    if let Ok(rd) = std::fs::read_dir(&base) {
        for entry in rd.flatten() {
            if let Ok(meta) = entry.metadata() {
                if meta.is_dir() {
                    if let Ok(modified) = meta.modified() {
                        if latest.as_ref().map(|(_, t)| modified > *t).unwrap_or(true) {
                            latest = Some((entry.path(), modified));
                        }
                    }
                }
            }
        }
    }
    latest.map(|(p, _)| p).unwrap_or(base)
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(id: &str, title: &str, parent: &str, created: &str) -> EntryMeta {
        EntryMeta {
            id: id.into(),
            title: title.into(),
            content_head: String::new(),
            kind: "context".into(),
            importance: "normal".into(),
            tags: Vec::new(),
            parent_id: parent.into(),
            created_at: created.into(),
            updated_at: created.into(),
            recall_count: 0,
            deleted: false,
        }
    }

    #[test]
    fn tags_split() {
        assert_eq!(split_tags("a, b,,c"), vec!["a", "b", "c"]);
        assert!(split_tags("  ").is_empty());
    }

    #[test]
    fn root_and_branch_resolution() {
        let mut entries = HashMap::new();
        entries.insert("root".into(), meta("root", "编程开发", "", "2026-01-01"));
        entries.insert("mid".into(), meta("mid", "子主题", "root", "2026-01-02"));
        entries.insert("leaf".into(), meta("leaf", "叶子", "mid", "2026-01-03"));
        let (roots, map) = build_roots(&entries);
        assert!(roots.iter().any(|r| r.title == "编程开发"));
        let (idx, cat, branch) = resolve_root(entries.get("leaf").unwrap(), &entries, &map);
        assert_eq!(cat, "编程开发");
        assert_eq!(branch, "mid");
        assert!(idx >= 0);
    }

    #[test]
    fn diary_group_resolution() {
        let mut entries = HashMap::new();
        entries.insert(
            "d1".into(),
            meta("d1", "活动轨迹 2026-09-17", "", "2026-09-17"),
        );
        entries.insert("d2".into(), meta("d2", "某天", "d1", "2026-09-18"));
        let (roots, map) = build_roots(&entries);
        assert!(roots.iter().any(|r| r.title == "日记" && r.index == 0));
        let (_idx, cat, branch) = resolve_root(entries.get("d2").unwrap(), &entries, &map);
        assert_eq!(cat, "日记");
        // d2 是根 d1 的直接子节点，它自己就是该分支
        assert_eq!(branch, "d2");
    }

    #[test]
    fn uuid_shape() {
        assert!(looks_like_uuid("5e78ea23-bf31-46c8-beb9-3245ccda2e94"));
        assert!(!looks_like_uuid("5e78ea23"));
        assert!(!looks_like_uuid("zzzzzzzz-bf31-46c8-beb9-3245ccda2e94"));
    }

    #[test]
    fn recall_ids_parse() {
        let raw = r#"["5e78ea23-bf31-46c8-beb9-3245ccda2e94","bad","5e78ea23-bf31-46c8-beb9-3245ccda2e94"]"#;
        let ids = parse_id_list(raw);
        assert_eq!(ids.len(), 1);
    }
}
