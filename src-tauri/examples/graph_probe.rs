//! 图谱数据层干跑：cargo run --example graph_probe
//!
//! 只读验证：打开 rsrs 库 → 载入 m3 向量 → 投影 → 输出节点/边/根统计；
//! 然后读一次增量游标（无新事件时应为 0 ops）。
use std::path::PathBuf;
use std::sync::Arc;

use memtools_lib::graph::{self, GraphService};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut since: Option<(i64, i64)> = None;
    let mut wait_secs: u64 = 0;
    let mut json_out: Option<PathBuf> = None;
    let mut data_arg: Option<PathBuf> = None;
    let mut i = 1;
    while i < args.len() {
        if args[i] == "--since" && i + 2 < args.len() {
            since = Some((
                args[i + 1].parse().unwrap_or(0),
                args[i + 2].parse().unwrap_or(0),
            ));
            i += 3;
            continue;
        }
        if args[i] == "--wait" && i + 1 < args.len() {
            wait_secs = args[i + 1].parse().unwrap_or(0);
            i += 2;
            continue;
        }
        if args[i] == "--json" && i + 1 < args.len() {
            json_out = Some(PathBuf::from(&args[i + 1]));
            i += 2;
            continue;
        }
        data_arg = Some(PathBuf::from(&args[i]));
        i += 1;
    }
    let data_dir = match data_arg.or_else(graph::data_dir_override) {
        Some(p) => p,
        None => {
            eprintln!("需要数据目录参数，或设置 RSRS_DATA_DIR 指向账号数据目录");
            std::process::exit(2);
        }
    };
    let cache_dir = std::env::temp_dir().join("memtools-graph-probe");
    println!("data_dir = {}", data_dir.display());
    println!("cache_dir = {}", cache_dir.display());

    let svc = Arc::new(GraphService::new(cache_dir));
    let t = std::time::Instant::now();
    let snap = match svc.snapshot(&data_dir) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("SNAPSHOT FAILED: {e}");
            std::process::exit(1);
        }
    };
    println!(
        "snapshot: nodes={} links={} roots={} vectors={} missing={} recall_max={} basis_cached={} ms={} note={:?}",
        snap.stats.nodes,
        snap.stats.links,
        snap.roots.len(),
        snap.stats.vectors,
        snap.stats.missing_vectors,
        snap.stats.recall_max,
        snap.stats.basis_cached,
        snap.stats.ms,
        snap.stats.note
    );
    println!("cursor: seq={} qid={}", snap.cursor.seq, snap.cursor.qid);
    if let Some(path) = &json_out {
        match serde_json::to_string(&snap) {
            Ok(text) => match std::fs::write(path, text) {
                Ok(_) => println!("snapshot json -> {}", path.display()),
                Err(e) => eprintln!("write json failed: {e}"),
            },
            Err(e) => eprintln!("serialize snapshot failed: {e}"),
        }
    }
    if let Some(n) = snap.nodes.first() {
        println!(
            "first node: id={} name={} cat={} root_index={} x={:.1} y={:.1} z={:.1}",
            n.id, n.name, n.category, n.root_index, n.x, n.y, n.z
        );
    }
    let cats = {
        let mut m: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for n in &snap.nodes {
            *m.entry(n.category.clone()).or_default() += 1;
        }
        let mut v: Vec<_> = m.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1));
        v.truncate(12);
        v
    };
    println!("top categories: {cats:?}");

    // 第二次加载应命中缓存
    let t2 = std::time::Instant::now();
    let snap2 = svc.snapshot(&data_dir).expect("second snapshot");
    println!(
        "second snapshot: vectors={} basis_cached={} ms={} (elapsed {:.2}s)",
        snap2.stats.vectors,
        snap2.stats.basis_cached,
        snap2.stats.ms,
        t2.elapsed().as_secs_f64()
    );

    if wait_secs > 0 {
        println!(
            "waiting {}s for external changes (snapshot cursor seq={} qid={})…",
            wait_secs, snap.cursor.seq, snap.cursor.qid
        );
        std::thread::sleep(std::time::Duration::from_secs(wait_secs));
    }

    // 增量：默认从当前游标起（应无事件）；--since seq qid 可验证历史增量
    let (s, q) = since.unwrap_or((snap.cursor.seq, snap.cursor.qid));
    match svc.events_since(s, q) {
        Ok(e) => {
            println!(
                "events since ({s},{q}): ops={} reset={} has_more={} cursor=({},{})",
                e.ops.len(),
                e.reset,
                e.has_more,
                e.cursor.seq,
                e.cursor.qid
            );
            for op in e.ops.iter().take(8) {
                println!(
                    "  op={} id={:?} ids={:?} parent={:?} link={:?} query={:?}",
                    op.op, op.id, op.ids, op.parent_id, op.link, op.query
                );
            }
        }
        Err(e) => println!("events error: {e}"),
    }

    println!("total elapsed: {:.2}s", t.elapsed().as_secs_f64());
}
