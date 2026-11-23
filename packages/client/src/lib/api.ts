import { url } from './base';

export class ApiError extends Error {
  constructor(
    public status: number,
    message: string,
  ) {
    super(message);
  }
}

async function call<T>(method: string, path: string, body?: unknown): Promise<T> {
  const init: RequestInit = { method, credentials: 'same-origin', headers: {} };
  if (body instanceof FormData) init.body = body;
  else if (body !== undefined) {
    init.body = JSON.stringify(body);
    (init.headers as Record<string, string>)['Content-Type'] = 'application/json';
  }
  const res = await fetch(url(path), init);
  const text = await res.text();
  let data: unknown = null;
  try {
    data = text ? JSON.parse(text) : null;
  } catch {
    data = text;
  }
  if (!res.ok) throw new ApiError(res.status, (data as { error?: string })?.error || res.statusText || 'Request failed');
  return data as T;
}

export const api = {
  get: <T>(p: string) => call<T>('GET', p),
  post: <T>(p: string, b?: unknown) => call<T>('POST', p, b ?? {}),
  put: <T>(p: string, b?: unknown) => call<T>('PUT', p, b ?? {}),
  patch: <T>(p: string, b?: unknown) => call<T>('PATCH', p, b ?? {}),
  del: <T>(p: string) => call<T>('DELETE', p),
  upload: async (file: File, kind: 'playmat' | 'card' | 'override' | 'sleeve'): Promise<number> => {
    if (file.size > 10 * 1024 * 1024) throw new ApiError(413, 'Images are limited to 10 MB');
    const fd = new FormData();
    fd.append('kind', kind);
    fd.append('file', file);
    const r = await call<{ id: number }>('POST', 'api/assets', fd);
    return r.id;
  },
};

export async function download(path: string, filename: string, body?: unknown) {
  const res = await fetch(url(path), body ? { method: 'POST', credentials: 'same-origin', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) } : { credentials: 'same-origin' });
  if (!res.ok) {
    let msg = res.statusText;
    try {
      msg = (await res.json()).error || msg;
    } catch {
      /* ignore */
    }
    throw new ApiError(res.status, msg);
  }
  const blob = await res.blob();
  saveBlob(blob, filename);
}

export function saveBlob(blob: Blob, filename: string) {
  const a = document.createElement('a');
  a.href = URL.createObjectURL(blob);
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  setTimeout(() => {
    URL.revokeObjectURL(a.href);
    a.remove();
  }, 1000);
}
