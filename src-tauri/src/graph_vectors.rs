//! 图谱向量层：只读 rsrs 的 core-index m3 向量，做确定性 PCA 投影。
//!
//! 数据来源（与 mind3D bridge 相同，全部只读）：
//!   - SQLite 表 core_artifacts.memory_id → artifact = "rsi1:<hash>"
//!   - <data_dir>/core-index/<hash> = zlib(JSON){ model:"m3", document:[f32;1024], ... }
//!
//! 缓存：
//!   - {cache_dir}/vectors.f32 + vectors.meta.json：按 id 增量复用，避免每次新增记忆都全量重读
//!   - {cache_dir}/basis.json：冻结 PCA 基座（新节点投影到同一基，整图不重排）
//!
//! 红线：不写 rsrs 数据目录；只打开只读 SQLite、只读 core-index 文件。

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::Path;

use flate2::read::ZlibDecoder;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

pub const DIM: usize = 1024;
const TARGET_RADIUS: f32 = 700.0;
const CACHE_VERSION: u32 = 1;

#[derive(Debug, Default, Clone)]
pub struct Vectors {
    pub ids: Vec<String>,
    pub data: Vec<f32>,
    pub index: HashMap<String, usize>,
}

impl Vectors {
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn get(&self, id: &str) -> Option<&[f32]> {
        self.index
            .get(id)
            .and_then(|&i| self.data.get(i * DIM..(i + 1) * DIM))
    }
}

#[derive(Debug, Clone)]
pub struct Basis {
    pub mean: Vec<f32>,
    pub components: Vec<Vec<f32>>,
    pub scale: f32,
    pub count: usize,
}

#[derive(Debug, Serialize, Deserialize)]
struct CacheMeta {
    version: u32,
    dim: usize,
    generated_at: String,
    entries: Vec<CacheEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CacheEntry {
    id: String,
    hash: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct BasisJson {
    version: u32,
    dim: usize,
    count: usize,
    radius: f32,
    scale: f32,
    mean: Vec<f32>,
    components: Vec<Vec<f32>>,
    generated_at: String,
}

#[derive(Debug, Deserialize)]
struct ArtifactJson {
    #[serde(default)]
    model: String,
    #[serde(default)]
    document: Vec<f32>,
}

/// 读 core_artifacts 的 artifact 文本（"rsi1:<hash>"）里的 hash。
fn decode_artifact(blob: &[u8]) -> String {
    let s = String::from_utf8_lossy(blob);
    let t = s.trim();
    match t.split_once(':') {
        Some((_, hash)) => hash.trim().to_string(),
        None => t.to_string(),
    }
}

/// 读取单条 m3 向量工件；非 m3 / 缺失 / 损坏返回 None。
fn read_m3_document(core_dir: &Path, hash: &str) -> Option<Vec<f32>> {
    if hash.is_empty() {
        return None;
    }
    let path = core_dir.join(hash);
    let bytes = fs::read(path).ok()?;
    let mut out = Vec::new();
    let mut decoder = ZlibDecoder::new(&bytes[..]);
    decoder.read_to_end(&mut out).ok()?;
    let art: ArtifactJson = serde_json::from_slice(&out).ok()?;
    if art.model != "m3" || art.document.len() != DIM {
        return None;
    }
    Some(art.document)
}

/// 当前 core_artifacts 的 id → [hash] 映射（一个记忆可能有多代工件）。
fn read_artifacts(conn: &Connection) -> Result<HashMap<String, Vec<String>>, String> {
    let mut stmt = conn
        .prepare("SELECT memory_id, artifact FROM core_artifacts")
        .map_err(|e| format!("读取 core_artifacts 失败: {e}"))?;
    let rows = stmt
        .query_map([], |r| {
            let id: String = r.get(0)?;
            let blob: Vec<u8> = r.get(1)?;
            Ok((id, decode_artifact(&blob)))
        })
        .map_err(|e| format!("查询 core_artifacts 失败: {e}"))?;
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    for row in rows {
        let (id, hash) = row.map_err(|e| format!("解析 core_artifacts 行失败: {e}"))?;
        if id.is_empty() || hash.is_empty() {
            continue;
        }
        let list = map.entry(id).or_default();
        if !list.iter().any(|h| h == &hash) {
            list.push(hash);
        }
    }
    Ok(map)
}

fn live_ids(conn: &Connection) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT id FROM memories WHERE deleted = 0")
        .map_err(|e| format!("查询 memories 失败: {e}"))?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| format!("读取 live ids 失败: {e}"))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("解析 memory id 失败: {e}"))?);
    }
    Ok(out)
}

fn read_cache(cache_dir: &Path) -> (HashMap<String, (String, Vec<f32>)>, bool) {
    let meta_path = cache_dir.join("vectors.meta.json");
    let bin_path = cache_dir.join("vectors.f32");
    let meta_text = match fs::read_to_string(&meta_path) {
        Ok(s) => s,
        Err(_) => return (HashMap::new(), false),
    };
    let meta: CacheMeta = match serde_json::from_str(&meta_text) {
        Ok(m) => m,
        Err(_) => return (HashMap::new(), false),
    };
    if meta.version != CACHE_VERSION || meta.dim != DIM {
        return (HashMap::new(), false);
    }
    let bytes = match fs::read(&bin_path) {
        Ok(b) => b,
        Err(_) => return (HashMap::new(), false),
    };
    let n = meta.entries.len();
    if bytes.len() != n * DIM * 4 {
        return (HashMap::new(), false);
    }
    let mut map = HashMap::with_capacity(n);
    for (i, e) in meta.entries.into_iter().enumerate() {
        let off = i * DIM * 4;
        let mut v = Vec::with_capacity(DIM);
        for c in bytes[off..off + DIM * 4].chunks_exact(4) {
            v.push(f32::from_le_bytes([c[0], c[1], c[2], c[3]]));
        }
        map.insert(e.id, (e.hash, v));
    }
    (map, true)
}

fn write_cache(
    cache_dir: &Path,
    ids: &[String],
    hashes: &[String],
    data: &[f32],
) -> Result<(), String> {
    fs::create_dir_all(cache_dir).map_err(|e| format!("创建向量缓存目录失败: {e}"))?;
    let mut bytes = Vec::with_capacity(data.len() * 4);
    for v in data {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    fs::write(cache_dir.join("vectors.f32"), &bytes)
        .map_err(|e| format!("写入 vectors.f32 失败: {e}"))?;
    let meta = CacheMeta {
        version: CACHE_VERSION,
        dim: DIM,
        generated_at: chrono_like_now(),
        entries: ids
            .iter()
            .zip(hashes.iter())
            .map(|(id, hash)| CacheEntry {
                id: id.clone(),
                hash: hash.clone(),
            })
            .collect(),
    };
    let text = serde_json::to_string(&meta).map_err(|e| format!("序列化向量缓存失败: {e}"))?;
    fs::write(cache_dir.join("vectors.meta.json"), text)
        .map_err(|e| format!("写入 vectors.meta.json 失败: {e}"))?;
    Ok(())
}

/// 加载全量 m3 向量（只保留存活记忆）。命中缓存时只补新增 id。
///
/// 返回 `(vectors, artifacts, missing, note)`：
///   - artifacts 交给调用方保存，实时增量读新节点向量时复用
///   - missing = 存活但没有 m3 向量的条数
pub fn load_vectors(
    conn: &Connection,
    core_dir: &Path,
    cache_dir: &Path,
) -> (Vectors, HashMap<String, Vec<String>>, usize, Option<String>) {
    let artifacts = match read_artifacts(conn) {
        Ok(a) => a,
        Err(e) => {
            return (
                Vectors::default(),
                HashMap::new(),
                0,
                Some(e),
            )
        }
    };
    let live = match live_ids(conn) {
        Ok(l) => l,
        Err(e) => {
            return (Vectors::default(), artifacts, 0, Some(e));
        }
    };
    if live.is_empty() {
        return (Vectors::default(), artifacts, 0, None);
    }
    if !core_dir.is_dir() {
        return (
            Vectors::default(),
            artifacts,
            live.len(),
            Some(format!(
                "core-index 目录不存在：{}（图谱将使用父子树兜底布局）",
                core_dir.display()
            )),
        );
    }

    let (cached, cache_ok) = read_cache(cache_dir);
    let mut ids: Vec<String> = Vec::with_capacity(live.len());
    let mut data: Vec<f32> = Vec::with_capacity(live.len() * DIM);
    let mut hashes: Vec<String> = Vec::with_capacity(live.len());
    let mut index: HashMap<String, usize> = HashMap::with_capacity(live.len());
    let mut missing = 0usize;
    let mut changed = !cache_ok;

    for id in &live {
        let current_hashes = artifacts.get(id);
        // ① 缓存命中，且使用的工件仍在当前工件表中 → 直接复用
        if let Some((hash, v)) = cached.get(id) {
            if current_hashes
                .map(|hs| hs.iter().any(|h| h == hash))
                .unwrap_or(false)
                && v.len() == DIM
            {
                index.insert(id.clone(), ids.len());
                ids.push(id.clone());
                data.extend_from_slice(v);
                hashes.push(hash.clone());
                continue;
            }
        }
        // ② 未命中：逐个工件尝试读 m3
        let mut found: Option<(String, Vec<f32>)> = None;
        if let Some(hs) = current_hashes {
            for h in hs {
                if let Some(v) = read_m3_document(core_dir, h) {
                    found = Some((h.clone(), v));
                    break;
                }
            }
        }
        match found {
            Some((h, v)) => {
                index.insert(id.clone(), ids.len());
                ids.push(id.clone());
                data.extend_from_slice(&v);
                hashes.push(h);
                changed = true;
            }
            None => {
                missing += 1;
            }
        }
    }

    // 缓存的 ids 顺序/内容与本次不一致 → 重写
    if !changed && (ids.len() != cached.len() || hashes.iter().any(|h| !cached.values().any(|(ch, _)| ch == h))) {
        changed = true;
    }
    let vectors = Vectors { ids, data, index };
    if changed {
        if let Err(e) = write_cache(cache_dir, &vectors.ids, &hashes, &vectors.data) {
            return (vectors, artifacts, missing, Some(e));
        }
    }
    let note = if missing > 0 {
        Some(format!("{missing} 条记忆没有 m3 向量（已用位置兜底）"))
    } else {
        None
    };
    (vectors, artifacts, missing, note)
}

/// 读取单条记忆的 m3 向量（实时新增节点用）。优先用调用方传入的工件表。
pub fn read_vector_for_id(
    conn: &Connection,
    core_dir: &Path,
    id: &str,
    artifacts: Option<&HashMap<String, Vec<String>>>,
) -> Option<Vec<f32>> {
    let owned;
    let hashes = match artifacts.and_then(|a| a.get(id)) {
        Some(hs) => hs,
        None => {
            let mut stmt = conn
                .prepare("SELECT artifact FROM core_artifacts WHERE memory_id = ?1")
                .ok()?;
            let rows = stmt
                .query_map([id], |r| r.get::<_, Vec<u8>>(0))
                .ok()?;
            owned = rows
                .filter_map(|r| r.ok())
                .map(|b| decode_artifact(&b))
                .filter(|h| !h.is_empty())
                .collect::<Vec<_>>();
            &owned
        }
    };
    for h in hashes {
        if let Some(v) = read_m3_document(core_dir, h) {
            return Some(v);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// PCA 基座
// ---------------------------------------------------------------------------

fn mulberry32(seed: u32) -> impl FnMut() -> f32 {
    let mut a = seed;
    move || {
        a = a.wrapping_add(0x6d2b79f5);
        let mut t = a;
        t = (t ^ (t >> 15)).wrapping_mul(1 | t);
        t = (t.wrapping_add((t ^ (t >> 7)).wrapping_mul(61 | t))) ^ t;
        ((t ^ (t >> 14)) as f64 / 4294967296.0) as f32
    }
}

fn normalize(v: &mut [f32]) {
    let mut s = 0f32;
    for x in v.iter() {
        s += *x * *x;
    }
    let s = s.sqrt();
    if s > 1e-12 {
        for x in v.iter_mut() {
            *x /= s;
        }
    }
}

fn deterministic_start(seed: u32) -> Vec<f32> {
    let mut rnd = mulberry32(seed);
    let mut v: Vec<f32> = (0..DIM).map(|_| rnd() * 2.0 - 1.0).collect();
    normalize(&mut v);
    v
}

fn mean_of(vectors: &Vectors) -> Vec<f32> {
    let n = vectors.len();
    let mut mean = vec![0f32; DIM];
    if n == 0 {
        return mean;
    }
    for i in 0..n {
        let off = i * DIM;
        for d in 0..DIM {
            mean[d] += vectors.data[off + d];
        }
    }
    for d in 0..DIM {
        mean[d] /= n as f32;
    }
    mean
}

/// 均匀抽样（确定性）：基座只需要方向，不必把全部 1300 条都放进幂迭代。
fn sample_data(vectors: &Vectors, max: usize) -> (Vec<f32>, usize) {
    let n = vectors.len();
    if n <= max {
        return (vectors.data.clone(), n);
    }
    let mut out = Vec::with_capacity(max * DIM);
    for i in 0..max {
        let idx = i * n / max;
        out.extend_from_slice(&vectors.data[idx * DIM..(idx + 1) * DIM]);
    }
    (out, max)
}

fn centered_data(data: &[f32], mean: &[f32], n: usize) -> Vec<f32> {
    let mut m = data.to_vec();
    for i in 0..n {
        let off = i * DIM;
        for d in 0..DIM {
            m[off + d] -= mean[d];
        }
    }
    m
}

fn power_iteration(m: &[f32], n: usize, iters: usize, seed: u32) -> Vec<f32> {
    let mut v = deterministic_start(seed);
    let mut tmp = vec![0f32; n];
    let mut w = vec![0f32; DIM];
    for _ in 0..iters {
        for i in 0..n {
            let off = i * DIM;
            let mut s = 0f32;
            for d in 0..DIM {
                s += m[off + d] * v[d];
            }
            tmp[i] = s;
        }
        w.fill(0.0);
        for i in 0..n {
            let t = tmp[i];
            if t == 0.0 {
                continue;
            }
            let off = i * DIM;
            for d in 0..DIM {
                w[d] += m[off + d] * t;
            }
        }
        normalize(&mut w);
        v.copy_from_slice(&w);
    }
    // 符号规整：最大绝对值分量为正，保证同一输入输出稳定
    let mut mi = 0;
    let mut mv = -1.0f32;
    for (i, x) in v.iter().enumerate() {
        let a = x.abs();
        if a > mv {
            mv = a;
            mi = i;
        }
    }
    if v[mi] < 0.0 {
        for x in v.iter_mut() {
            *x = -*x;
        }
    }
    v
}

fn deflate(m: &mut [f32], n: usize, v: &[f32]) {
    for i in 0..n {
        let off = i * DIM;
        let mut s = 0f32;
        for d in 0..DIM {
            s += m[off + d] * v[d];
        }
        for d in 0..DIM {
            m[off + d] -= s * v[d];
        }
    }
}

fn build_basis(vectors: &Vectors) -> Basis {
    let n = vectors.len();
    if n == 0 {
        return Basis {
            mean: vec![0.0; DIM],
            components: vec![vec![0.0; DIM]; 3],
            scale: 1.0,
            count: 0,
        };
    }
    let mean = mean_of(vectors);
    // 基座用抽样数据做幂迭代：1300 条全量在 debug 下太慢；
    // 投影仍用全部向量，布局观感基本一致。
    const MAX_SAMPLE: usize = 480;
    let (sample, sn) = sample_data(vectors, MAX_SAMPLE);
    let mut m = centered_data(&sample, &mean, sn);
    let iters = if cfg!(debug_assertions) { 24 } else { 60 };
    let mut components: Vec<Vec<f32>> = Vec::with_capacity(3);
    for k in 0..3u32 {
        let seed = 0x9e37_79b9u32.wrapping_add(k.wrapping_mul(0x85eb_ca6b));
        let v = power_iteration(&m, sn, iters, seed);
        components.push(v.clone());
        deflate(&mut m, sn, &v);
    }
    // 用全部向量（未消融的原始中心化数据）计算缩放
    let mut max_r2 = 0f32;
    for i in 0..n {
        let off = i * DIM;
        let mut r2 = 0f32;
        for c in &components {
            let mut s = 0f32;
            for d in 0..DIM {
                s += (vectors.data[off + d] - mean[d]) * c[d];
            }
            r2 += s * s;
        }
        if r2 > max_r2 {
            max_r2 = r2;
        }
    }
    let scale = if max_r2 > 0.0 {
        TARGET_RADIUS / max_r2.sqrt()
    } else {
        1.0
    };
    Basis {
        mean,
        components,
        scale,
        count: n,
    }
}

pub fn load_or_build_basis(cache_dir: &Path, vectors: &Vectors, force: bool) -> (Basis, bool) {
    let path = cache_dir.join("basis.json");
    if !force {
        if let Ok(text) = fs::read_to_string(&path) {
            if let Ok(j) = serde_json::from_str::<BasisJson>(&text) {
                let ok = j.version == CACHE_VERSION
                    && j.dim == DIM
                    && j.mean.len() == DIM
                    && j.components.len() == 3
                    && j.components.iter().all(|c| c.len() == DIM)
                    && j.scale.is_finite()
                    && j.scale > 0.0;
                if ok {
                    return (
                        Basis {
                            mean: j.mean,
                            components: j.components,
                            scale: j.scale,
                            count: j.count,
                        },
                        true,
                    );
                }
            }
        }
    }
    let basis = build_basis(vectors);
    let _ = fs::create_dir_all(cache_dir);
    let j = BasisJson {
        version: CACHE_VERSION,
        dim: DIM,
        count: basis.count,
        radius: TARGET_RADIUS,
        scale: basis.scale,
        mean: basis.mean.clone(),
        components: basis.components.clone(),
        generated_at: chrono_like_now(),
    };
    if let Ok(text) = serde_json::to_string(&j) {
        let _ = fs::write(&path, text);
    }
    (basis, false)
}

pub fn project(vec: Option<&[f32]>, basis: &Basis) -> [f32; 3] {
    let Some(v) = vec else {
        return [0.0, 0.0, 0.0];
    };
    if v.len() != DIM {
        return [0.0, 0.0, 0.0];
    }
    let mut out = [0f32; 3];
    for k in 0..3 {
        let c = &basis.components[k];
        let mut s = 0f32;
        for d in 0..DIM {
            s += (v[d] - basis.mean[d]) * c[d];
        }
        out[k] = s * basis.scale;
    }
    out
}

pub fn project_all(vectors: &Vectors, basis: &Basis) -> HashMap<String, [f32; 3]> {
    let mut out = HashMap::with_capacity(vectors.len());
    for id in &vectors.ids {
        out.insert(id.clone(), project(vectors.get(id), basis));
    }
    out
}

/// 无 chrono 依赖的 ISO 时间（UTC），够用于缓存元数据。
pub fn chrono_like_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // 简单换算 civil date（足够缓存标记，不做复杂时区）
    let days = secs / 86400;
    let rem = secs % 86400;
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days as i64 + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_artifact_hash() {
        assert_eq!(decode_artifact(b"rsi1:abc123"), "abc123");
        assert_eq!(decode_artifact(b"  plain  "), "plain");
    }

    #[test]
    fn project_zero_basis() {
        let basis = Basis {
            mean: vec![0.0; DIM],
            components: vec![vec![0.0; DIM]; 3],
            scale: 1.0,
            count: 0,
        };
        assert_eq!(project(Some(&vec![1.0; DIM]), &basis), [0.0, 0.0, 0.0]);
        assert_eq!(project(None, &basis), [0.0, 0.0, 0.0]);
    }
}
