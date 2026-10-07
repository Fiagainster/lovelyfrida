/**
 * 高频观测事件批量层（O-02 前置）：
 * probe_hit / probe_error / dlopen / register_natives 逐条 send() 时，每事件付出
 * 一次 JS→C 序列化 + 调度成本，下游 sidecar/Rust/前端四层管线也被逐条打满。
 * 这里做环形缓冲 + 双阈值（256 条或 100ms）flush：单条 send 变 {t:"batch", items}。
 * hello / pong / console / ssl_data（data 参数通道）不走此层。
 *
 * 批次⑪③：
 * - 每条事件盖章递增 `aseq`（agent seq；区别于宿主侧 TraceRecord.seq）——宿主 trace
 *   管线据此检测 agent→sidecar→宿主段的丢失（断裂补 seq_gap 记录进证据文件）。
 *   重排/重发不重盖：aseq 代表「agent 产生的顺序」，乱序到达由宿主按 ≤已见最大值处理。
 * - flush 失败（script 卸载瞬间 send 抛出）不再静默丢批：退回队头有界重试（3 次），
 *   仍失败则放弃——渠道已死时进程内无处可写，丢失由宿主侧 seq_gap 兜底留痕。
 */
type Batchable = Record<string, unknown>;

const buf: Batchable[] = [];
let timer: ReturnType<typeof setTimeout> | null = null;
/** agent 事件序号（每事件递增；不随 flush 失败重置） */
let aseqCounter = 0;
/** 连续 flush 失败次数（成功即清零；≥3 视为渠道已死，放弃重试） */
let failStreak = 0;

/** 条数阈值：达到立即 flush（洪峰下把批量大小钉死，避免单条消息过大） */
const MAX_ITEMS = 256;
/** 时间阈值：低频事件的延迟上限（ms） */
const FLUSH_MS = 100;
/** 重试退避基数（failStreak 递增）与上限 */
const MAX_FAIL_STREAK = 3;
/** 重排积压上限：渠道垂死时丢最旧保最新（防无界内存） */
const MAX_REQUEUE_ITEMS = MAX_ITEMS * 8;

function flush(): void {
  timer = null;
  if (!buf.length) {
    failStreak = 0;
    return;
  }
  const items = buf.splice(0, buf.length);
  try {
    send({ t: "batch", items } as unknown as { [key: string]: unknown });
    failStreak = 0;
  } catch {
    // 此前这一步先 splice 后 send，失败即整批静默丢失且无任何留痕（批次⑪③）。
    buf.unshift(...items);
    if (buf.length > MAX_REQUEUE_ITEMS) {
      buf.splice(0, buf.length - MAX_REQUEUE_ITEMS);
    }
    if (failStreak < MAX_FAIL_STREAK) {
      failStreak++;
      if (timer === null) {
        timer = setTimeout(flush, FLUSH_MS * failStreak);
      }
    }
    // 连续失败达上限：放弃重试。aseq 断裂由宿主补 seq_gap 记录，证据链可解释。
  }
}

/** 高频观测事件的唯一入口（替代直接 send） */
export function emitEvent(payload: Batchable): void {
  aseqCounter++;
  payload["aseq"] = aseqCounter;
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

/** 测试观察用：当前缓冲中的事件数 */
export function _pendingForTest(): number {
  return buf.length;
}
