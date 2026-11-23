// Minimal structural diff for pushing per-viewer game views. Arrays are replaced whole.

export type PatchOp = { p: (string | number)[]; v?: unknown; d?: 1 };

function isObj(x: unknown): x is Record<string, unknown> {
  return typeof x === 'object' && x !== null && !Array.isArray(x);
}

function arrEq(a: unknown[], b: unknown[]): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    const x = a[i];
    const y = b[i];
    if (x === y) continue;
    if (typeof x !== 'object' || typeof y !== 'object' || x === null || y === null) return false;
    if (JSON.stringify(x) !== JSON.stringify(y)) return false;
  }
  return true;
}

export function diff(a: unknown, b: unknown, path: (string | number)[] = [], out: PatchOp[] = []): PatchOp[] {
  if (a === b) return out;
  if (isObj(a) && isObj(b)) {
    for (const k of Object.keys(a)) if (!(k in b)) out.push({ p: [...path, k], d: 1 });
    for (const k of Object.keys(b)) {
      if (!(k in a)) out.push({ p: [...path, k], v: b[k] });
      else diff(a[k], b[k], [...path, k], out);
    }
    return out;
  }
  if (Array.isArray(a) && Array.isArray(b) && arrEq(a, b)) return out;
  out.push({ p: path, v: b });
  return out;
}

/** Applies ops immutably along the touched paths (so React sees new references). */
export function applyPatch<T>(root: T, ops: PatchOp[]): T {
  let r: unknown = root;
  for (const op of ops) {
    if (!op.p.length) {
      r = op.v;
      continue;
    }
    const top: Record<string, unknown> = Array.isArray(r) ? ([...r] as unknown as Record<string, unknown>) : { ...(r as Record<string, unknown>) };
    let node: Record<string, unknown> = top;
    for (let i = 0; i < op.p.length - 1; i++) {
      const k = op.p[i] as string;
      const child = node[k];
      const copy = Array.isArray(child) ? [...child] : { ...(child as Record<string, unknown>) };
      node[k] = copy;
      node = copy as Record<string, unknown>;
    }
    const last = op.p[op.p.length - 1] as string;
    if (op.d) delete node[last];
    else node[last] = op.v;
    r = top;
  }
  return r as T;
}
