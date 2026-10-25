// memTools 3D 图谱视图
// - 布局：后端把 m3 向量 PCA 投影到 3D（固定坐标），本文件只负责渲染
// - 视觉：色相←分类根；大小←召回热度；透明度/辉光←重要度；形状←kind
// - 实时：main.js 轮询 graph_events_since，本文件 applyOps 增量更新
import ForceGraph3D from "3d-force-graph";
import * as THREE from "three";
import containerPlyUrl from "./assets/brain2.container.ply?url";
import { buildContainerVolume } from "./containerVolume.js";
import { pickProjectedNode, isGraphClick } from "./interaction.js";

const GOLDEN_ANGLE = 137.508;
const FOCUS_HOLD_MS = 6000;
const FOCUS_LEVELS_DEFAULT = 3;
const FOCUS_LEVELS_MIN = 1;
const FOCUS_LEVELS_MAX = 6;
// 单次聚焦最多点亮的节点数：整层放不下时少显示一层，避免一个根把整棵树点亮
const FOCUS_MAX_NODES_DEFAULT = 20;
const FOCUS_MAX_NODES_MIN = 1;
const FOCUS_MAX_NODES_MAX = 500;
const FOCUS_DECAY = 0.55; // 层级亮度衰减：b = decay^lv，0 层最亮
const FADE_MS = 300; // 焦点消失后的渐暗时长（demo6 同款）
const HOVER_DEBOUNCE_MS = 50; // 悬停去抖，避免快速划过时反复做 BFS
const DIM_NODE_FACTOR = 0.18; // 非焦点节点：基准不透明度 × 0.18（半透明可见，不消失）
const DIM_LINK_OPACITY = 0.05; // 非焦点边残留不透明度
const DIM_LINK_DARKEN = 0.65; // 非焦点边颜色向黑压暗的比例
const FOCUS_LINK_COLOR = "#9fe4ff"; // 被点亮父子边的高亮色（demo6 同款）
const CORE_LINK_FOCUS_COLOR = "#63c9d8"; // 被点亮共现边的弱高亮色
const CONTAINER_FILL = 1.04; // 容器比节点包围盒略大，确保节点落在里面
const CONTAINER_VOLUME_RES = 88; // 体素分辨率（最长轴格数）
const CONTAINER_VOLUME_DILATE = 1; // 壳膨胀层数（闭合脑面小孔）
const CONTAINER_SHRINK = 0.93; // 越界节点向容器中心收缩系数
const CONTAINER_SHRINK_TRIES = 90;
const CONTAINER_OPACITY = 0.16;
const CONTAINER_POINT_RATIO = 0.004; // 点大小 = 场景最大跨度 × 该比例
const WHITE = new THREE.Color(0xffffff);

const KIND_HUE = {
  context: 200,
  skill: 150,
  task: 45,
  knowledge: 270,
  decision: 20,
  preference: 320,
  emotion: 330,
  time: 180,
};

const KIND_SHAPE = {
  context: "sphere",
  skill: "octahedron",
  task: "box",
  knowledge: "dodecahedron",
  decision: "torus",
  preference: "cone",
  emotion: "icosahedron",
  time: "ring",
};

const IMP_RANK = { important: 1, normal: 0.5, trivial: 0 };
const IMP_OPACITY = { important: 0.95, normal: 0.82, trivial: 0.6 };
const IMP_GLOW = { important: 1.6, normal: 1.0, trivial: 0.6 };

const LINK_STYLE = {
  parent: { color: "#4d7bd6", width: 1.25, opacity: 0.46 },
  corecall: { color: "#2fb6a6", width: 0.7, opacity: 0.18 },
};

const NODE_BASE_RADIUS = 3.4;
const geomCache = new Map();

function geometryFor(shape) {
  if (geomCache.has(shape)) return geomCache.get(shape);
  let g;
  switch (shape) {
    case "box":
      g = new THREE.BoxGeometry(1.4, 1.4, 1.4);
      break;
    case "cone":
      g = new THREE.ConeGeometry(1, 2, 20);
      break;
    case "torus":
      g = new THREE.TorusGeometry(0.9, 0.35, 14, 28);
      break;
    case "ring":
      g = new THREE.TorusGeometry(1, 0.18, 12, 32);
      break;
    case "octahedron":
      g = new THREE.OctahedronGeometry(1.2);
      break;
    case "dodecahedron":
      g = new THREE.DodecahedronGeometry(1.1);
      break;
    case "icosahedron":
      g = new THREE.IcosahedronGeometry(1.15);
      break;
    case "capsule":
      g = new THREE.CapsuleGeometry(0.7, 1.1, 6, 14);
      break;
    case "sphere":
    default:
      g = new THREE.SphereGeometry(1, 18, 14);
      break;
  }
  geomCache.set(shape, g);
  return g;
}

function normalizeImportance(v) {
  const s = String(v || "").toLowerCase();
  return s === "important" || s === "trivial" ? s : "normal";
}

function maxLevelOf(map) {
  let max = 0;
  if (map) for (const lv of map.values()) if (lv > max) max = lv;
  return max;
}

function darkenHex(hex, k) {
  const n = parseInt(String(hex || "#4d7bd6").replace("#", ""), 16);
  if (!Number.isFinite(n)) return hex;
  const f = Math.max(0, Math.min(1, k));
  const r = Math.round(((n >> 16) & 255) * (1 - f));
  const g = Math.round(((n >> 8) & 255) * (1 - f));
  const b = Math.round((n & 255) * (1 - f));
  return `#${((1 << 24) | (r << 16) | (g << 8) | b).toString(16).slice(1)}`;
}

// 两个 hex 颜色按 t 线性混合（t=0 → a，t=1 → b），用于焦点渐暗时高亮色平滑回到基色。
function mixHex(a, b, t) {
  const na = parseInt(String(a || "#000000").replace("#", ""), 16);
  const nb = parseInt(String(b || "#000000").replace("#", ""), 16);
  if (!Number.isFinite(na) || !Number.isFinite(nb)) return a;
  const k = Math.max(0, Math.min(1, t));
  const ar = (na >> 16) & 255;
  const ag = (na >> 8) & 255;
  const ab = na & 255;
  const br = (nb >> 16) & 255;
  const bg = (nb >> 8) & 255;
  const bb = nb & 255;
  const r = Math.round(ar + (br - ar) * k);
  const g = Math.round(ag + (bg - ag) * k);
  const bl = Math.round(ab + (bb - ab) * k);
  return `#${((1 << 24) | (r << 16) | (g << 8) | bl).toString(16).slice(1)}`;
}

function hslToHex(h, s, l) {  h = ((h % 360) + 360) % 360;
  const c = (1 - Math.abs(2 * l - 1)) * s;
  const x = c * (1 - Math.abs(((h / 60) % 2) - 1));
  const m = l - c / 2;
  let r = 0;
  let g = 0;
  let b = 0;
  if (h < 60) {
    r = c;
    g = x;
  } else if (h < 120) {
    r = x;
    g = c;
  } else if (h < 180) {
    g = c;
    b = x;
  } else if (h < 240) {
    g = x;
    b = c;
  } else if (h < 300) {
    r = x;
    b = c;
  } else {
    r = c;
    b = x;
  }
  const to = (v) => Math.round((v + m) * 255).toString(16).padStart(2, "0");
  return `#${to(r)}${to(g)}${to(b)}`;
}

// 与后端一致：同根下按分支大小给 ±45° 色相微差，避免「编程开发」一色铺满。
function computeBranchShades(nodes) {
  const byRoot = new Map();
  for (const n of nodes) {
    const idx = Number(n.root_index);
    if (!Number.isInteger(idx) || idx < 0) continue;
    if (!byRoot.has(idx)) byRoot.set(idx, new Map());
    const m = byRoot.get(idx);
    const bid = String(n.branch_id || "");
    m.set(bid, (m.get(bid) || 0) + 1);
  }
  const shades = new Map();
  for (const m of byRoot.values()) {
    [...m.entries()]
      .sort((a, b) => b[1] - a[1])
      .forEach(([bid], i) => shades.set(bid, ((i % 7) - 3) * 15));
  }
  return shades;
}

function nodeBounds(nodes) {
  const min = [Infinity, Infinity, Infinity];
  const max = [-Infinity, -Infinity, -Infinity];
  for (const n of nodes) {
    const p = [Number(n.x) || 0, Number(n.y) || 0, Number(n.z) || 0];
    for (let i = 0; i < 3; i++) {
      if (p[i] < min[i]) min[i] = p[i];
      if (p[i] > max[i]) max[i] = p[i];
    }
  }
  if (!Number.isFinite(min[0])) return null;
  return {
    min,
    max,
    size: [max[0] - min[0], max[1] - min[1], max[2] - min[2]],
    center: [(max[0] + min[0]) / 2, (max[1] + min[1]) / 2, (max[2] + min[2]) / 2],
  };
}

const PLY_TYPE_SIZE = {
  char: 1, uchar: 1, int8: 1, uint8: 1,
  short: 2, ushort: 2, int16: 2, uint16: 2,
  int: 4, uint: 4, int32: 4, uint32: 4, float: 4, float32: 4,
  double: 8, float64: 8,
};

function parsePly(arrayBuffer) {
  const bytes = new Uint8Array(arrayBuffer);
  const headText = new TextDecoder("ascii").decode(
    bytes.subarray(0, Math.min(bytes.length, 8192))
  );
  const endIdx = headText.indexOf("end_header");
  if (endIdx < 0) throw new Error("PLY 缺少 end_header");

  let offset = endIdx + "end_header".length;
  if (bytes[offset] === 0x0d) offset += 1;
  if (bytes[offset] === 0x0a) offset += 1;

  const header = headText.slice(0, endIdx);
  let format = "ascii";
  let count = 0;
  let inVertex = false;
  const props = [];
  for (const raw of header.split(/\r?\n/)) {
    const t = raw.trim().split(/\s+/);
    if (t[0] === "format") format = t[1] || format;
    else if (t[0] === "element") {
      inVertex = t[1] === "vertex";
      if (inVertex) count = Number(t[2]) || 0;
    } else if (t[0] === "property" && inVertex) {
      if (t[1] === "list") throw new Error("不支持 PLY 的 list 属性");
      props.push({ type: t[1], name: t[2] });
    }
  }
  if (!count || !props.length) throw new Error("PLY 未声明 vertex 属性");

  const positions = new Float32Array(count * 3);
  const min = [Infinity, Infinity, Infinity];
  const max = [-Infinity, -Infinity, -Infinity];
  const push = (i, x, y, z) => {
    positions[i * 3] = x;
    positions[i * 3 + 1] = y;
    positions[i * 3 + 2] = z;
    [x, y, z].forEach((v, k) => {
      if (v < min[k]) min[k] = v;
      if (v > max[k]) max[k] = v;
    });
  };

  if (format === "ascii") {
    const tokens = new TextDecoder("ascii")
      .decode(bytes.subarray(offset))
      .trim()
      .split(/\s+/);
    const ix = props.findIndex((p) => p.name === "x");
    const iy = props.findIndex((p) => p.name === "y");
    const iz = props.findIndex((p) => p.name === "z");
    if (ix < 0 || iy < 0 || iz < 0) throw new Error("PLY 缺少 x/y/z");
    for (let i = 0; i < count; i++) {
      const base = i * props.length;
      push(
        i,
        Number(tokens[base + ix]),
        Number(tokens[base + iy]),
        Number(tokens[base + iz])
      );
    }
  } else if (format === "binary_little_endian") {
    const view = new DataView(arrayBuffer);
    let stride = 0;
    const offs = {};
    for (const p of props) {
      offs[p.name] = { off: stride, type: p.type };
      stride += PLY_TYPE_SIZE[p.type] || 0;
    }
    if (!stride) throw new Error("PLY 属性类型未知");
    if (offset + stride * count > bytes.length) throw new Error("PLY 数据长度不足");
    const read = (at, type) => {
      switch (type) {
        case "float": case "float32": return view.getFloat32(at, true);
        case "double": case "float64": return view.getFloat64(at, true);
        case "char": case "int8": return view.getInt8(at);
        case "uchar": case "uint8": return view.getUint8(at);
        case "short": case "int16": return view.getInt16(at, true);
        case "ushort": case "uint16": return view.getUint16(at, true);
        case "int": case "int32": return view.getInt32(at, true);
        case "uint": case "uint32": return view.getUint32(at, true);
        default: return 0;
      }
    };
    const px = offs.x, py = offs.y, pz = offs.z;
    if (!px || !py || !pz) throw new Error("PLY 缺少 x/y/z");
    for (let i = 0; i < count; i++) {
      const at = offset + i * stride;
      push(
        i,
        read(at + px.off, px.type),
        read(at + py.off, py.type),
        read(at + pz.off, pz.type)
      );
    }
  } else {
    throw new Error(`不支持的 PLY format：${format}`);
  }

  return { count, positions, min, max };
}

export class GraphView {
  constructor(container, opts = {}) {
    this.container = container;
    this.onNodeClick = opts.onNodeClick || (() => {});
    this.onStatus = opts.onStatus || (() => {});
    this.nodes = [];
    this.links = [];
    this.nodeById = new Map();
    this.linkById = new Map();
    this.objects = new Map();
    this.focusIds = new Set();
    this.focusTimer = null;
    this.hoverId = null;
    this.hoverTimer = null;
    this.pinnedId = null;
    this._fade = null;
    this._fadeK = 0;
    this._fadeRaf = 0;
    this.levels = FOCUS_LEVELS_DEFAULT;
    this.maxNodes = FOCUS_MAX_NODES_DEFAULT;
    this.effectiveLevels = 0;
    this.levelMap = null;
    this._adj = null;
    this.recallMax = 1;
    this.shadeByBranch = new Map();
    this.active = false;
    this.containerVisible = opts.containerVisible !== false;
    this.containerPoints = null;
    this.containerInfo = null;
    this.containerVolume = null;
    this.containerFrame = null;
    this.containerOutside = 0;
    this.containerError = "";
    this.idleDim = opts.idleDim !== false;
    this._containerTask = null;
    this._initGraph();
    this._loadContainer();
  }

  _initGraph() {
    const graph = new ForceGraph3D(this.container, {
      controlType: "orbit",
      rendererConfig: { antialias: true, alpha: true },
    })
      .backgroundColor("#080b12")
      .showNavInfo(false)
      .nodeId("id")
      .enablePointerInteraction(false)
      .enableNodeDrag(false)
      .nodeThreeObject((n) => this._nodeObject(n))
      .nodeThreeObjectExtend(false)
      .linkColor((l) => l.color)
      .linkWidth((l) => l.width)
      .linkOpacity((l) => l.opacity)
      .linkDirectionalArrowLength((l) => (l.directed ? 3.5 : 0))
      .linkDirectionalArrowRelPos(0.9)
      .linkDirectionalArrowColor((l) => l.color);
    this.graph = graph;
    this._initPicking();

    // 坐标全部固定，不需要力导向迭代
    graph.cooldownTicks(0);

    const scene = graph.scene();
    scene.add(new THREE.AmbientLight(0xffffff, 0.75));
    const d1 = new THREE.DirectionalLight(0xffffff, 1.05);
    d1.position.set(1, 1, 1);
    scene.add(d1);
    const d2 = new THREE.DirectionalLight(0x6ea8ff, 0.55);
    d2.position.set(-1, -1, -1);
    scene.add(d2);

    const controls = graph.controls();
    if (controls) {
      controls.enableDamping = true;
      controls.dampingFactor = 0.08;
      controls.rotateSpeed = 0.72;
      controls.zoomSpeed = 0.9;
      controls.autoRotateSpeed = 0.6;
    }

    this._ro = new ResizeObserver(() => this.resize());
    this._ro.observe(this.container);
    this.resize();
    this.graph.pauseAnimation();
  }

  /** 屏幕拾取独立于异步 hover；保留 OrbitControls 的旋转、缩放和触摸处理。 */
  _initPicking() {
    const canvas = this.graph.renderer().domElement;
    const tooltip = document.createElement("div");
    tooltip.className = "graph-node-tooltip hidden";
    tooltip.setAttribute("role", "tooltip");
    this.container.appendChild(tooltip);
    this.tooltip = tooltip;
    const point = event => ({ x: event.clientX, y: event.clientY,
      pointerId: event.pointerId, button: event.button });
    const down = event => {
      this._pointerStart = event.button === 0 && event.isPrimary !== false
        ? { ...point(event), maxDistance: 0 } : null;
      this._hideTooltip();
    };
    const move = event => {
      if (this._pointerStart) {
        const start = this._pointerStart;
        start.maxDistance = Math.max(start.maxDistance,
          Math.hypot(start.x - event.clientX, start.y - event.clientY));
        return;
      }
      const node = this._pickAtEvent(event);
      this.container.style.cursor = node ? "pointer" : "grab";
      this._setHover(node?.id);
      tooltip.classList.toggle("hidden", !node);
      if (node) {
        tooltip.textContent = this._label(node);
        const rect = this.container.getBoundingClientRect();
        tooltip.style.left = Math.max(4, Math.min(event.clientX - rect.left + 14, rect.width - 280)) + "px";
        tooltip.style.top = Math.max(4, Math.min(event.clientY - rect.top + 14, rect.height - 100)) + "px";
      }
    };
    const up = event => {
      const start = this._pointerStart;
      this._pointerStart = null;
      if (isGraphClick(start, point(event))) this.selectNode(this._pickAtEvent(event)?.id ?? null);
    };
    const leave = () => { this._hideTooltip(); this._setHover(null); };
    const cancel = () => { this._pointerStart = null; leave(); };
    this._pointerHandlers = { pointerdown: down, pointermove: move, pointerup: up,
      pointerleave: leave, pointercancel: cancel };
    for (const [type, listener] of Object.entries(this._pointerHandlers))
      canvas.addEventListener(type, listener, { capture: true });
  }

  _hideTooltip() {
    this.tooltip?.classList.add("hidden");
  }

  /** 投影后的最小点击半径为 CSS 像素，不随缩放或显示器像素密度缩小。 */
  _pickAtEvent(event) {
    if (!this.active) return null;
    const rect = this.graph.renderer().domElement.getBoundingClientRect();
    const pointer = { x: event.clientX - rect.left, y: event.clientY - rect.top };
    if (pointer.x < 0 || pointer.y < 0 || pointer.x > rect.width || pointer.y > rect.height) return null;
    const camera = this.graph.camera();
    camera.updateMatrixWorld();
    const position = new THREE.Vector3();
    const view = new THREE.Vector3();
    const projected = new THREE.Vector3();
    const scale = rect.height / (2 * Math.tan(THREE.MathUtils.degToRad(camera.fov / 2)));
    const points = [];
    for (const node of this.nodes) {
      position.set(node.x, node.y, node.z);
      view.copy(position).applyMatrix4(camera.matrixWorldInverse);
      projected.copy(position).project(camera);
      if (view.z >= 0 || projected.z < -1 || projected.z > 1) continue;
      const radius = this.objects.get(node.id)?.userData.mesh.scale.x ?? NODE_BASE_RADIUS;
      points.push({ node, x: (projected.x + 1) * rect.width / 2,
        y: (1 - projected.y) * rect.height / 2, z: -view.z, radius: radius * scale / -view.z });
    }
    return pickProjectedNode(points, pointer);
  }

  /** 锁定详情并暂停自转；搜索定位可将相机朝同一节点平滑拉近。 */
  selectNode(id, { moveCamera = false } = {}) {
    const node = id == null ? null : this.nodeById.get(String(id));
    if (id != null && !node) return false;
    clearTimeout(this.hoverTimer);
    this.hoverId = null;
    this._hideTooltip();
    if (!node) {
      this.clearFocus();
      this.onNodeClick(null);
      return true;
    }
    this.setAutoRotate(false);
    this.onStatus({ autoRotate: false });
    this.pin(node.id);
    this.onNodeClick(node);
    if (moveCamera) {
      const controls = this.graph.controls();
      const direction = this.graph.camera().position.clone().sub(controls.target);
      if (!direction.lengthSq()) direction.set(0, 0.15, 1);
      direction.normalize().multiplyScalar(Math.max(120, NODE_BASE_RADIUS * 36));
      this.graph.cameraPosition({ x: node.x + direction.x, y: node.y + direction.y,
        z: node.z + direction.z }, { x: node.x, y: node.y, z: node.z }, 500);
    }
    return true;
  }

  resize() {
    if (!this.graph) return;
    const w = Math.max(1, this.container.clientWidth);
    const h = Math.max(1, this.container.clientHeight);
    this.graph.width(w).height(h);
  }

  setActive(active) {
    this.active = !!active;
    if (!this.graph) return;
    if (this.active) {
      this.resize();
      this.graph.resumeAnimation();
    } else {
      this.graph.pauseAnimation();
      this._pointerStart = null;
      this._hideTooltip();
    }
  }

  setAutoRotate(on) {
    const c = this.graph?.controls?.();
    if (c) c.autoRotate = !!on;
  }

  resetView() {
    if (!this.graph) return;
    this.graph.cameraPosition(
      { x: 0, y: 180, z: 1500 },
      { x: 0, y: 0, z: 0 },
      700
    );
  }

  zoomToFit() {
    if (!this.graph) return;
    if (this.container.clientWidth < 2 || this.container.clientHeight < 2) {
      clearTimeout(this._fitTimer);
      this._fitTimer = setTimeout(() => this.zoomToFit(), 200);
      return;
    }
    this.graph.zoomToFit(700, 40);
  }

  /** 全量替换（首次加载 / 重载 / 重排）。 */
  setData(snapshot) {
    const rawNodes = Array.isArray(snapshot?.nodes) ? snapshot.nodes : [];
    const rawLinks = Array.isArray(snapshot?.links) ? snapshot.links : [];
    this.recallMax = 1;
    for (const n of rawNodes) {
      const c = Number(n.recall_count) || 0;
      if (c > this.recallMax) this.recallMax = c;
    }
    this.shadeByBranch = computeBranchShades(rawNodes);
    this.nodes = rawNodes.map((n) => this._decorateNode(n));
    this.nodeById = new Map(this.nodes.map((n) => [n.id, n]));
    this.links = rawLinks.map((l) => this._decorateLink(l));
    this.linkById = new Map(this.links.map((l) => [l.id, l]));
    this.objects.clear();
    this.focusIds = new Set();
    this.levelMap = null;
    this._adj = null;
    clearTimeout(this.focusTimer);
    this.focusTimer = null;
    clearTimeout(this.hoverTimer);
    this.hoverTimer = null;
    this.hoverId = null;
    this.pinnedId = null;
    this.onNodeClick(null);
    this._hideTooltip();
    this._cancelFade();

    this._applyContainerFit();
    this.graph.graphData({ nodes: this.nodes, links: this.links });
    this._applyFocus();
    this.resetView();
    setTimeout(() => this.zoomToFit(), 80);
    setTimeout(() => {
      this.resize();
      this.zoomToFit();
    }, 420);
    this._emitStatus();
  }

  /** 增量应用后端 op（node.add/update/remove、link.add、relink、focus）。 */
  applyOps(ops) {
    if (!Array.isArray(ops) || !ops.length) return;
    let structural = false;
    const focusIds = [];

    for (const ev of ops) {
      if (!ev || !ev.op) continue;
      if (ev.op === "node.add" || ev.op === "node.update") {
        const payload = ev.data;
        if (!payload?.id) continue;
        const n = this._decorateNode(payload);
        const old = this.nodeById.get(n.id);
        if (old) {
          Object.assign(old, n);
          this._placeNode(old);
          this._refreshNodeObject(old);
          // 节点更新时按其最新 parent_id 重挂父边
          this._relinkParent(n.id, n.parent_id);
          structural = true;
        } else {
          this._placeNode(n);
          this.nodes.push(n);
          this.nodeById.set(n.id, n);
          structural = true;
          if (ev.op === "node.add") focusIds.push(n.id);
        }
      } else if (ev.op === "node.remove") {
        const id = String(ev.id || "");
        if (!id) continue;
        this.nodes = this.nodes.filter((n) => n.id !== id);
        this.nodeById.delete(id);
        if (this.pinnedId === id) this.selectNode(null);
        if (this.hoverId === id) { this.hoverId = null; this._hideTooltip(); }
        this.links = this.links.filter((l) => {
          const s = l.source?.id ?? l.source;
          const t = l.target?.id ?? l.target;
          return s !== id && t !== id;
        });
        this.linkById = new Map(this.links.map((l) => [l.id, l]));
        this.objects.delete(id);
        structural = true;
      } else if (ev.op === "link.add") {
        const raw = ev.link;
        if (!raw?.id) continue;
        const l = this._decorateLink(raw);
        const s = l.source?.id ?? l.source;
        const t = l.target?.id ?? l.target;
        if (!this.linkById.has(l.id) && this.nodeById.has(s) && this.nodeById.has(t)) {
          this.links.push(l);
          this.linkById.set(l.id, l);
          structural = true;
        }
      } else if (ev.op === "relink") {
        const id = String(ev.id || "");
        if (!id) continue;
        this._relinkParent(id, String(ev.parent_id || ""));
        structural = true;
      } else if (ev.op === "focus") {
        if (Array.isArray(ev.ids)) focusIds.push(...ev.ids.map(String));
      }
    }

    if (structural) {
      // 坐标固定（fx/fy/fz），graphData 只做增删 diff，不会重排
      this.graph.graphData({ nodes: this.nodes, links: this.links });
      this._adj = null;
      if (this.levelMap) {
        const roots = this._roots();
        this.levelMap = roots.length ? this._computeLevels(roots) : null;
      }
      this._applyFocus();
    }
    if (focusIds.length) this.focus(focusIds);
    this._emitStatus();
  }

  _relinkParent(id, parentId) {
    this.links = this.links.filter((l) => {
      if (l.type !== "parent") return true;
      const s = l.source?.id ?? l.source;
      const t = l.target?.id ?? l.target;
      return s !== id && t !== id;
    });
    if (parentId && this.nodeById.has(parentId) && this.nodeById.has(id)) {
      const l = this._decorateLink({
        id: `p_${parentId}_${id}`,
        source: parentId,
        target: id,
        type: "parent",
      });
      this.links.push(l);
    }
    this.linkById = new Map(this.links.map((l) => [l.id, l]));
  }

  _decorateNode(n) {
    const kind = String(n.kind || "context").toLowerCase();
    const importance = normalizeImportance(n.importance);
    const rank = IMP_RANK[importance] ?? 0.5;
    const recall = Number(n.recall_count) || 0;
    const heat =
      this.recallMax > 0
        ? Math.log1p(recall) / Math.log1p(this.recallMax)
        : 0;
    const rootIndex = Number.isInteger(Number(n.root_index))
      ? Number(n.root_index)
      : -1;
    const baseHue =
      rootIndex >= 0
        ? (rootIndex * GOLDEN_ANGLE) % 360
        : KIND_HUE[kind] ?? 200;
    const shade =
      rootIndex >= 0 ? this.shadeByBranch.get(String(n.branch_id || "")) || 0 : 0;
    const color = hslToHex(
      baseHue + shade,
      0.55 + 0.35 * rank,
      0.5 + 0.12 * heat
    );
    const x = Number(n.x) || 0;
    const y = Number(n.y) || 0;
    const z = Number(n.z) || 0;
    return {
      ...n,
      id: String(n.id),
      kind,
      importance,
      category: n.category || `其他·${kind}`,
      color,
      val: 1 + 3.5 * heat,
      opacity: IMP_OPACITY[importance] ?? 0.82,
      glow: IMP_GLOW[importance] ?? 1.0,
      shape: KIND_SHAPE[kind] || "sphere",
      raw: { x, y, z },
      // 冻结投影坐标 → 3d-force-graph 固定节点
      x,
      y,
      z,
      fx: x,
      fy: y,
      fz: z,
    };
  }

  _decorateLink(l) {
    const type = l.type || "parent";
    const style = LINK_STYLE[type] || LINK_STYLE.parent;
    const n = Number(l.n) || 1;
    const width =
      type === "corecall"
        ? Math.min(2.4, style.width + 0.35 * Math.log1p(n))
        : style.width;
    return {
      ...l,
      id: String(l.id),
      source: l.source?.id ?? l.source,
      target: l.target?.id ?? l.target,
      type,
      directed: type === "parent",
      baseColor: style.color,
      baseOpacity: style.opacity,
      color: style.color,
      width,
      opacity: style.opacity,
    };
  }

  _nodeStyle(n, map = this.levelMap, fadeK = this._fadeK) {
    const baseOp = n.opacity ?? 0.82;
    const baseEm = 0.35 * (n.glow || 1);
    const baseR = Math.cbrt(Math.max(0.0001, n.val || 1)) * NODE_BASE_RADIUS;
    const idle = this.idleDim
      ? {
          op: Math.max(0.1, baseOp * DIM_NODE_FACTOR),
          em: baseEm * 0.12,
          radius: baseR * 0.9,
          white: 0,
        }
      : { op: baseOp, em: baseEm, radius: baseR, white: 0 };
    if (!map) return idle;

    const lv = map.get(n.id);
    let focus;
    if (lv !== undefined && lv <= this.levels) {
      const b = Math.pow(FOCUS_DECAY, lv);
      focus = {
        op: Math.min(1, baseOp * (0.55 + 0.55 * b)),
        em: baseEm * (0.7 + 5.5 * b),
        radius: baseR * (1 + 0.55 * b),
        white: 0.35 * b,
      };
    } else {
      focus = {
        op: Math.max(0.1, baseOp * DIM_NODE_FACTOR),
        em: baseEm * 0.12,
        radius: baseR * 0.9,
        white: 0,
      };
    }
    if (!fadeK) return focus;

    const k = Math.max(0, Math.min(1, fadeK));
    return {
      op: focus.op + (idle.op - focus.op) * k,
      em: focus.em + (idle.em - focus.em) * k,
      radius: focus.radius + (idle.radius - focus.radius) * k,
      white: focus.white * (1 - k),
    };
  }

  _paintNode(n, group, st = this._nodeStyle(n)) {
    const ud = group.userData;
    ud.mesh.scale.setScalar(st.radius);
    ud.mat.opacity = st.op;
    ud.mat.depthWrite = st.op > 0.5;
    ud.mat.emissiveIntensity = st.em;
    ud.mat.color.set(ud.baseColor || n.color);
    ud.mat.emissive.set(ud.baseColor || n.color);
    if (st.white > 0) {
      ud.mat.color.lerp(WHITE, st.white);
      ud.mat.emissive.lerp(WHITE, st.white);
    }
  }

  _nodeObject(n) {
    const st = this._nodeStyle(n);
    const baseColor = n.color || "#43e8ff";
    const mat = new THREE.MeshStandardMaterial({
      color: new THREE.Color(baseColor),
      emissive: new THREE.Color(baseColor),
      emissiveIntensity: st.em,
      metalness: 0.26,
      roughness: 0.42,
      transparent: true,
      opacity: st.op,
      depthWrite: st.op > 0.5,
    });
    const mesh = new THREE.Mesh(geometryFor(n.shape), mat);
    mesh.scale.setScalar(st.radius);
    const group = new THREE.Group();
    group.add(mesh);
    group.userData = {
      nodeId: n.id,
      mesh,
      mat,
      shape: n.shape,
      baseColor,
      baseRadius: Math.cbrt(Math.max(0.0001, n.val || 1)) * NODE_BASE_RADIUS,
      baseOpacity: n.opacity ?? 0.82,
      baseGlow: n.glow ?? 1,
    };
    if (st.white > 0) {
      mat.color.lerp(WHITE, st.white);
      mat.emissive.lerp(WHITE, st.white);
    }
    this.objects.set(n.id, group);
    return group;
  }

  _refreshNodeObject(n) {
    const group = this.objects.get(n.id);
    if (!group) return;
    const ud = group.userData;
    if (ud.shape !== n.shape) {
      ud.mesh.geometry = geometryFor(n.shape);
      ud.shape = n.shape;
    }
    ud.baseColor = n.color || ud.baseColor;
    ud.baseRadius = Math.cbrt(Math.max(0.0001, n.val || 1)) * NODE_BASE_RADIUS;
    ud.baseOpacity = n.opacity ?? 0.82;
    ud.baseGlow = n.glow ?? 1;
    this._paintNode(n, group);
  }

  setLevels(n) {
    const v = Math.max(
      FOCUS_LEVELS_MIN,
      Math.min(FOCUS_LEVELS_MAX, Math.round(Number(n) || FOCUS_LEVELS_DEFAULT))
    );
    if (v === this.levels) return;
    this.levels = v;
    this._recomputeFocus();
  }

  setMaxNodes(n) {
    const v = Math.max(
      FOCUS_MAX_NODES_MIN,
      Math.min(FOCUS_MAX_NODES_MAX, Math.round(Number(n) || FOCUS_MAX_NODES_DEFAULT))
    );
    if (v === this.maxNodes) return;
    this.maxNodes = v;
    this._recomputeFocus();
  }

  _recomputeFocus() {
    const roots = this._roots();
    if (!roots.length) {
      this._emitFocusStatus(0);
      return;
    }
    this._cancelFade();
    this.levelMap = this._computeLevels(roots);
    this.effectiveLevels = maxLevelOf(this.levelMap);
    this._applyFocus();
    this._emitFocusStatus(roots.length);
  }

  _setHover(id) {
    const next = id && this.nodeById.has(id) ? id : null;
    if (next === this.hoverId) return;
    clearTimeout(this.hoverTimer);
    this.hoverTimer = setTimeout(
      () => {
        this.hoverTimer = null;
        this.hoverId = next;
        this._refreshFocus({ fade: true });
      },
      next ? HOVER_DEBOUNCE_MS : 0
    );
  }

  pin(id) {
    const next = id && this.nodeById.has(id) ? id : null;
    if (next === this.pinnedId) return;
    this.pinnedId = next;
    if (next) {
      clearTimeout(this.focusTimer);
      this.focusTimer = null;
      this.focusIds = new Set();
    }
    this._refreshFocus({ fade: true });
    this._emitFocusStatus(this._roots().length);
  }

  _roots() {
    if (this.pinnedId && this.nodeById.has(this.pinnedId)) return [this.pinnedId];
    if (this.hoverId && this.nodeById.has(this.hoverId)) return [this.hoverId];
    return [...this.focusIds].filter((id) => this.nodeById.has(id));
  }

  _refreshFocus({ fade = false } = {}) {
    const roots = this._roots();
    if (!roots.length) {
      this._cancelFade();
      this.effectiveLevels = 0;
      if (fade && this.levelMap && this.levelMap.size) this._startFade();
      else {
        this.levelMap = null;
        this._applyFocus();
      }
      this._emitFocusStatus(0);
      return;
    }
    this._cancelFade();
    this.levelMap = this._computeLevels(roots);
    this.effectiveLevels = maxLevelOf(this.levelMap);
    this._applyFocus();
    this._emitFocusStatus(roots.length);
  }

  _startFade() {
    const map = this.levelMap;
    if (!map || !map.size) {
      this.levelMap = null;
      this._applyFocus();
      return;
    }
    this.levelMap = null;
    this._fade = { map, t0: performance.now(), dur: FADE_MS };
    this._fadeK = 0;
    cancelAnimationFrame(this._fadeRaf);
    const step = () => {
      const f = this._fade;
      if (!f) return;
      const k = f.dur > 0 ? Math.min(1, (performance.now() - f.t0) / f.dur) : 1;
      this._fadeK = k;
      this._applyFocus(f.map, k);
      if (k < 1) {
        this._fadeRaf = requestAnimationFrame(step);
      } else {
        this._fade = null;
        this._fadeK = 0;
        this._fadeRaf = 0;
        this._applyFocus(null, 0);
      }
    };
    this._fadeRaf = requestAnimationFrame(step);
  }

  _cancelFade() {
    if (this._fadeRaf) cancelAnimationFrame(this._fadeRaf);
    this._fadeRaf = 0;
    this._fade = null;
    this._fadeK = 0;
  }

  _emitFocusStatus(rootCount) {
    const lit = this.levelMap ? this.levelMap.size : 0;
    const used = lit ? this.effectiveLevels : 0;
    this.onStatus({
      msg: lit
        ? `点亮 ${rootCount} 个根 · ${lit} 个节点 · ${used} 层（上限 ${this.maxNodes}）`
        : "",
      levels: this.levels,
      usedLevels: used,
      maxNodes: this.maxNodes,
      focus: lit,
    });
  }

  focus(ids, holdMs = FOCUS_HOLD_MS) {
    const roots = [
      ...new Set((ids || []).map(String).filter((id) => this.nodeById.has(id))),
    ];
    if (!roots.length) return;
    clearTimeout(this.focusTimer);
    this.focusIds = new Set(roots);
    this._refreshFocus({ fade: false });
    this.focusTimer = setTimeout(() => {
      this.focusTimer = null;
      this.focusIds = new Set();
      this._refreshFocus({ fade: true });
    }, holdMs);
  }

  clearFocus() {
    clearTimeout(this.focusTimer);
    this.focusTimer = null;
    clearTimeout(this.hoverTimer);
    this.hoverTimer = null;
    this.hoverId = null;
    this.pinnedId = null;
    this.focusIds = new Set();
    this._refreshFocus({ fade: true });
    this.onStatus({
      msg: "高亮已复位",
      levels: this.levels,
      maxNodes: this.maxNodes,
      usedLevels: 0,
    });
  }

  _applyFocus(map = this.levelMap, fadeK = this._fadeK) {
    for (const [id, group] of this.objects) {
      const node = this.nodeById.get(id);
      if (node) this._paintNode(node, group, this._nodeStyle(node, map, fadeK));
    }
    this._applyLinkFocus(map, fadeK);
  }

  _applyLinkFocus(map = this.levelMap, fadeK = this._fadeK) {
    const k = Math.max(0, Math.min(1, fadeK));
    for (const l of this.links) {
      const idleColor = this.idleDim
        ? darkenHex(l.baseColor, DIM_LINK_DARKEN)
        : l.baseColor;
      const idleOpacity = this.idleDim ? DIM_LINK_OPACITY : l.baseOpacity;
      if (!map) {
        l.color = idleColor;
        l.opacity = idleOpacity;
        continue;
      }
      const s = l.source?.id ?? l.source;
      const t = l.target?.id ?? l.target;
      const la = map.get(s);
      const lb = map.get(t);
      let lv;
      if (la === undefined && lb === undefined) lv = Infinity;
      else lv = Math.max(la === undefined ? Infinity : la, lb === undefined ? Infinity : lb);

      let focusColor;
      let focusOpacity;
      if (lv <= this.levels) {
        const b = Math.pow(FOCUS_DECAY, lv);
        if (l.type === "parent") {
          focusColor = FOCUS_LINK_COLOR;
          focusOpacity = 0.18 + 0.66 * b;
        } else {
          focusColor = CORE_LINK_FOCUS_COLOR;
          focusOpacity = 0.05 + 0.18 * b;
        }
      } else {
        focusColor = darkenHex(l.baseColor, DIM_LINK_DARKEN);
        focusOpacity = DIM_LINK_OPACITY;
      }
      if (k > 0) {
        l.color = mixHex(focusColor, idleColor, k);
        l.opacity = focusOpacity + (idleOpacity - focusOpacity) * k;
      } else {
        l.color = focusColor;
        l.opacity = focusOpacity;
      }
    }
    this.graph
      .linkColor((l) => l.color)
      .linkOpacity((l) => l.opacity);
  }

  _buildAdjacency() {
    const adj = new Map();
    for (const n of this.nodes) adj.set(n.id, []);
    for (const l of this.links) {
      const s = l.source?.id ?? l.source;
      const t = l.target?.id ?? l.target;
      if (s === t) continue;
      const a = adj.get(s);
      const b = adj.get(t);
      if (a) a.push(t);
      if (b) b.push(s);
    }
    this._adj = adj;
  }

  _computeLevels(roots) {
    if (!this._adj) this._buildAdjacency();
    const map = new Map();
    const adj = this._adj;
    if (!adj) return map;
    const limit = Math.max(1, this.maxNodes);
    let frontier = [];
    for (const r of roots) {
      if (map.size >= limit) break;
      if (adj.has(r) && !map.has(r)) {
        map.set(r, 0);
        frontier.push(r);
      }
    }
    for (let d = 1; d <= this.levels && frontier.length; d++) {
      const next = [];
      const seen = new Set();
      for (const id of frontier) {
        for (const nb of adj.get(id) || []) {
          if (map.has(nb) || seen.has(nb)) continue;
          seen.add(nb);
          next.push(nb);
        }
      }
      if (!next.length) break;
      if (map.size + next.length > limit) {
        // 整层放不下就不下探这一层；但若连第 1 层都放不下（超级枢纽节点），
        // 聚焦会只剩根节点，改为按上限截断该层，保证聚焦仍有意义。
        if (d === 1) {
          for (const nb of next) {
            if (map.size >= limit) break;
            map.set(nb, d);
          }
        }
        break;
      }
      for (const nb of next) map.set(nb, d);
      frontier = next;
    }
    return map;
  }

  async _loadContainer() {
    if (this._containerTask) return this._containerTask;
    this._containerTask = (async () => {
      const res = await fetch(containerPlyUrl, { cache: "force-cache" });
      if (!res.ok) throw new Error(`容器点云 HTTP ${res.status}`);
      const ply = parsePly(await res.arrayBuffer());
      const geo = new THREE.BufferGeometry();
      geo.setAttribute("position", new THREE.BufferAttribute(ply.positions, 3));
      const mat = new THREE.PointsMaterial({
        color: 0x86c2ff,
        size: 4,
        sizeAttenuation: true,
        transparent: true,
        opacity: CONTAINER_OPACITY,
        depthWrite: false,
        blending: THREE.AdditiveBlending,
      });
      const points = new THREE.Points(geo, mat);
      points.frustumCulled = false;
      points.visible = this.containerVisible;
      this.containerPoints = points;
      this.containerInfo = ply;
      this.containerVolume = buildContainerVolume(ply.positions, {
        resolution: CONTAINER_VOLUME_RES,
        dilate: CONTAINER_VOLUME_DILATE,
      });
      this.graph.scene().add(points);
      this._applyContainerFit();
      if (this.nodes.length) {
        this.graph.graphData({ nodes: this.nodes, links: this.links });
        this._applyFocus();
      }
      this._emitStatus({
        container: this.containerVisible,
        containerPoints: ply.count,
        containerMode: this.containerVolume.mode,
        containerOutside: this.containerOutside,
      });
    })().catch((e) => {
      this.containerError = String(e?.message || e);
      this._containerTask = null;
      this._emitStatus({ containerError: this.containerError });
    });
    return this._containerTask;
  }

  _applyContainerFit() {
    const points = this.containerPoints;
    const info = this.containerInfo;
    const volume = this.containerVolume;
    if (!points || !info || !volume || !this.nodes.length || !this.containerVisible) return;
    const nb = nodeBounds(this.nodes.map((n) => n.raw || n));
    if (!nb) {
      points.visible = false;
      return;
    }
    const cs = [
      info.max[0] - info.min[0],
      info.max[1] - info.min[1],
      info.max[2] - info.min[2],
    ];
    const scale =
      Math.max(nb.size[0] / cs[0], nb.size[1] / cs[1], nb.size[2] / cs[2]) *
      CONTAINER_FILL;
    const cc = [
      (info.min[0] + info.max[0]) / 2,
      (info.min[1] + info.max[1]) / 2,
      (info.min[2] + info.max[2]) / 2,
    ];
    const px = nb.center[0] - cc[0] * scale;
    const py = nb.center[1] - cc[1] * scale;
    const pz = nb.center[2] - cc[2] * scale;
    this.containerFrame = { scale, p: [px, py, pz], center: nb.center.slice() };
    points.scale.setScalar(scale);
    points.position.set(px, py, pz);
    const span = Math.max(nb.size[0], nb.size[1], nb.size[2]);
    points.material.size = Math.max(0.6, span * CONTAINER_POINT_RATIO);
    points.visible = true;
    let outside = 0;
    for (const n of this.nodes) {
      if (!this._placeNode(n)) outside++;
    }
    this.containerOutside = outside;
  }

  _placeNode(n) {
    const frame = this.containerFrame;
    const volume = this.containerVolume;
    const raw = n.raw || { x: n.x, y: n.y, z: n.z };
    const put = (x, y, z) => {
      n.x = x;
      n.y = y;
      n.z = z;
      n.fx = x;
      n.fy = y;
      n.fz = z;
      return this._inVolume(x, y, z);
    };
    if (!frame || !volume || volume.mode !== "volume" || !this.containerVisible) {
      return put(raw.x, raw.y, raw.z);
    }
    let x = raw.x;
    let y = raw.y;
    let z = raw.z;
    const c = frame.center;
    for (let i = 0; i < CONTAINER_SHRINK_TRIES && !this._inVolume(x, y, z); i++) {
      x = c[0] + (x - c[0]) * CONTAINER_SHRINK;
      y = c[1] + (y - c[1]) * CONTAINER_SHRINK;
      z = c[2] + (z - c[2]) * CONTAINER_SHRINK;
    }
    if (!this._inVolume(x, y, z)) {
      const rp = volume.randomPoint();
      x = frame.p[0] + rp.x * frame.scale;
      y = frame.p[1] + rp.y * frame.scale;
      z = frame.p[2] + rp.z * frame.scale;
    }
    return put(x, y, z);
  }

  _inVolume(x, y, z) {
    const frame = this.containerFrame;
    const volume = this.containerVolume;
    if (!frame || !volume) return true;
    return volume.contains(
      (x - frame.p[0]) / frame.scale,
      (y - frame.p[1]) / frame.scale,
      (z - frame.p[2]) / frame.scale
    );
  }

  _restoreRawAll() {
    for (const n of this.nodes) {
      const r = n.raw;
      if (!r) continue;
      n.x = r.x;
      n.y = r.y;
      n.z = r.z;
      n.fx = r.x;
      n.fy = r.y;
      n.fz = r.z;
    }
  }

  setContainerVisible(on) {
    this.containerVisible = !!on;
    if (this.containerVisible) {
      if (!this.containerPoints) this._loadContainer();
      else {
        this._applyContainerFit();
        if (this.nodes.length) {
          this.graph.graphData({ nodes: this.nodes, links: this.links });
          this._applyFocus();
        }
      }
    } else {
      this._restoreRawAll();
      if (this.nodes.length) {
        this.graph.graphData({ nodes: this.nodes, links: this.links });
        this._applyFocus();
      }
      if (this.containerPoints) this.containerPoints.visible = false;
    }
    this._emitStatus({
      container: this.containerVisible,
      containerOutside: this.containerOutside,
    });
  }

  setIdleDim(on) {
    const next = !!on;
    if (next === this.idleDim) return;
    this.idleDim = next;
    this._applyFocus();
    this._emitStatus({ idleDim: this.idleDim });
  }

  _label(n) {
    const lines = [n.name || n.id];
    lines.push(
      `${n.category || "未分类"} · ${n.kind || "context"} · 召回 ${
        n.recall_count || 0
      }`
    );
    if (Array.isArray(n.tags) && n.tags.length) {
      lines.push(n.tags.map((t) => `#${t}`).join(" "));
    }
    return lines.join("\n");
  }

  _emitStatus(extra = {}) {
    this.onStatus({
      msg: extra.msg || "",
      nodes: this.nodes.length,
      links: this.links.length,
      ...extra,
    });
  }

  dispose() {
    const canvas = this.graph?.renderer().domElement;
    for (const [type, listener] of Object.entries(this._pointerHandlers || {}))
      canvas?.removeEventListener(type, listener, { capture: true });
    clearTimeout(this.focusTimer);
    clearTimeout(this.hoverTimer);
    this._cancelFade();
    try {
      this._ro?.disconnect();
    } catch (_) {}
    try {
      this.containerPoints?.geometry?.dispose?.();
      this.containerPoints?.material?.dispose?.();
    } catch (_) {}
    try {
      this.graph?._destructor?.();
    } catch (_) {}
    this.container.innerHTML = "";
  }
}
