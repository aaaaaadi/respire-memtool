function clampInt(v, lo, hi) {
  const n = Math.round(Number(v));
  if (!Number.isFinite(n)) return lo;
  return n < lo ? lo : n > hi ? hi : n;
}

function dilate(src, nx, ny, nz, r) {
  const out = new Uint8Array(nx * ny * nz);
  for (let z = 0; z < nz; z++) {
    for (let y = 0; y < ny; y++) {
      for (let x = 0; x < nx; x++) {
        const k = (z * ny + y) * nx + x;
        if (src[k]) {
          out[k] = 1;
          continue;
        }
        let hit = false;
        for (let dz = -r; dz <= r && !hit; dz++) {
          const zz = z + dz;
          if (zz < 0 || zz >= nz) continue;
          for (let dy = -r; dy <= r && !hit; dy++) {
            const yy = y + dy;
            if (yy < 0 || yy >= ny) continue;
            const rowBase = (zz * ny + yy) * nx;
            for (let dx = -r; dx <= r; dx++) {
              const xx = x + dx;
              if (xx < 0 || xx >= nx) continue;
              if (src[rowBase + xx]) {
                hit = true;
                break;
              }
            }
          }
        }
        if (hit) out[k] = 1;
      }
    }
  }
  return out;
}

export function buildContainerVolume(positions, opts = {}) {
  const count = Math.max(1, (positions.length / 3) | 0);
  let minX = Infinity, minY = Infinity, minZ = Infinity;
  let maxX = -Infinity, maxY = -Infinity, maxZ = -Infinity;
  for (let i = 0; i < count; i++) {
    const x = positions[i * 3], y = positions[i * 3 + 1], z = positions[i * 3 + 2];
    if (x < minX) minX = x;
    if (x > maxX) maxX = x;
    if (y < minY) minY = y;
    if (y > maxY) maxY = y;
    if (z < minZ) minZ = z;
    if (z > maxZ) maxZ = z;
  }
  if (!Number.isFinite(minX)) {
    minX = minY = minZ = 0;
    maxX = maxY = maxZ = 1;
  }
  const spanX = Math.max(1e-6, maxX - minX);
  const spanY = Math.max(1e-6, maxY - minY);
  const spanZ = Math.max(1e-6, maxZ - minZ);
  const maxSpan = Math.max(spanX, spanY, spanZ);

  const res = clampInt(opts.resolution ?? 88, 16, 200);
  const dil = clampInt(opts.dilate ?? 1, 0, 3);
  const cell = maxSpan / res;
  const nx = Math.max(2, Math.floor(spanX / cell) + 1);
  const ny = Math.max(2, Math.floor(spanY / cell) + 1);
  const nz = Math.max(2, Math.floor(spanZ / cell) + 1);
  const cells = nx * ny * nz;
  const at = (x, y, z) => (z * ny + y) * nx + x;
  const cellX = (v) => Math.min(nx - 1, Math.max(0, Math.floor((v - minX) / cell)));
  const cellY = (v) => Math.min(ny - 1, Math.max(0, Math.floor((v - minY) / cell)));
  const cellZ = (v) => Math.min(nz - 1, Math.max(0, Math.floor((v - minZ) / cell)));

  const shell = new Uint8Array(cells);
  let shellCells = 0;
  for (let i = 0; i < count; i++) {
    const k = at(
      cellX(positions[i * 3]),
      cellY(positions[i * 3 + 1]),
      cellZ(positions[i * 3 + 2])
    );
    if (!shell[k]) {
      shell[k] = 1;
      shellCells++;
    }
  }

  const barrier = dil > 0 ? dilate(shell, nx, ny, nz, dil) : shell;
  const exterior = new Uint8Array(cells);
  const queue = new Int32Array(cells);
  let head = 0;
  let tail = 0;
  const push = (k) => {
    if (!barrier[k] && !exterior[k]) {
      exterior[k] = 1;
      queue[tail++] = k;
    }
  };
  for (let z = 0; z < nz; z++)
    for (let y = 0; y < ny; y++) {
      push(at(0, y, z));
      push(at(nx - 1, y, z));
    }
  for (let z = 0; z < nz; z++)
    for (let x = 0; x < nx; x++) {
      push(at(x, 0, z));
      push(at(x, ny - 1, z));
    }
  for (let y = 0; y < ny; y++)
    for (let x = 0; x < nx; x++) {
      push(at(x, y, 0));
      push(at(x, y, nz - 1));
    }
  const plane = nx * ny;
  while (head < tail) {
    const k = queue[head++];
    const x = k % nx;
    const y = ((k / nx) | 0) % ny;
    if (x > 0) push(k - 1);
    if (x < nx - 1) push(k + 1);
    if (y > 0) push(k - nx);
    if (y < ny - 1) push(k + nx);
    if (k >= plane) push(k - plane);
    if (k < cells - plane) push(k + plane);
  }

  const inVol = new Uint8Array(cells);
  let interiorCells = 0;
  let candCount = 0;
  for (let k = 0; k < cells; k++) {
    if (shell[k]) {
      inVol[k] = 1;
      candCount++;
    } else if (!barrier[k] && !exterior[k]) {
      inVol[k] = 1;
      interiorCells++;
      candCount++;
    }
  }

  const closed = interiorCells >= Math.max(64, shellCells * 0.05);
  const candidates = new Uint32Array(Math.max(1, candCount));
  {
    let ci = 0;
    for (let k = 0; k < cells; k++) if (inVol[k]) candidates[ci++] = k;
  }
  const center = [(minX + maxX) / 2, (minY + maxY) / 2, (minZ + maxZ) / 2];

  return {
    mode: closed ? "volume" : "surface",
    resolution: res,
    cell,
    nx,
    ny,
    nz,
    min: [minX, minY, minZ],
    max: [maxX, maxY, maxZ],
    shellCells,
    interiorCells,
    size: candCount,
    contains(x, y, z) {
      if (x < minX || x > maxX || y < minY || y > maxY || z < minZ || z > maxZ) return false;
      return inVol[at(cellX(x), cellY(y), cellZ(z))] === 1;
    },
    randomPoint(out) {
      const o = out || { x: 0, y: 0, z: 0 };
      if (!candCount) {
        o.x = center[0];
        o.y = center[1];
        o.z = center[2];
        return o;
      }
      const k = candidates[(Math.random() * candCount) | 0];
      const x = k % nx;
      const y = ((k / nx) | 0) % ny;
      const z = (k / (nx * ny)) | 0;
      o.x = minX + (x + 0.5) * cell;
      o.y = minY + (y + 0.5) * cell;
      o.z = minZ + (z + 0.5) * cell;
      return o;
    },
    center,
  };
}
