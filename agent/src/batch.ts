/**
 * 高频观测事件批量层（O-02 前置）：
 * probe_hit / probe_error / dlopen / register_natives 逐条 send() 时，每事件付出
 * 一次 JS→C 序列化 + 调度成本，下游 sidecar/Rust/前端四层管线也被逐条打满。
 * 这里做环形缓冲 + 双阈值（256 条或 100ms）flush：单条 send 变 {t:"batch", items}。
 * hello / pong / console / ssl_data（data 参数通道）不走此层。
 */
type Batchable = Record<string, unknown>;

const buf: Batchable[] = [];
let timer: ReturnType<typeof setTimeout> | null = null;

/** 条数阈值：达到立即 flush（洪峰下把批量大小钉死，避免单条消息过大） */
const MAX_ITEMS = 256;
/** 时间阈值：低频事件的延迟上限（ms） */
const FLUSH_MS = 100;

function flush(): void {
  timer = null;
  if (!buf.length) return;
  const items = buf.splice(0, buf.length);
  send({ t: "batch", items } as unknown as { [key: string]: unknown });
}

/** 高频观测事件的唯一入口（替代直接 send） */
export function emitEvent(payload: Batchable): void {
  buf.push(payload);
  if (buf.length >= MAX_ITEMS) {
    if (timer !== null) {
      clearTimeout(timer);
    }
    flush();
    return;
  }
  if (timer === null) {
    timer = setTimeout(flush, FLUSH_MS);
  }
}
