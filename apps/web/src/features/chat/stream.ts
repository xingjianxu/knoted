import { z } from "zod";
import type { QueryClientBundle } from "../../lib/query";

const frameLimit = 1_048_576;
const citation = z.object({
  id: z.uuid(),
  message_id: z.uuid(),
  source_type: z.enum(["note", "file", "task"]),
  source_id: z.uuid(),
  title: z.string(),
  location: z.string().nullable(),
  snippet: z.string(),
  source_state: z.enum(["current", "deleted", "archived", "missing"]),
});
const event = z.object({
  type: z.enum(["queued", "started", "delta", "citation", "completed", "failed", "cancelled"]),
  run_id: z.uuid(),
  seq: z.int().positive(),
  delta: z.string().nullish(),
  citation: citation.nullish(),
  error: z.object({ code: z.string() }).nullish(),
});

export type ChatEvent = z.infer<typeof event>;
export type TerminalEvent = "completed" | "failed" | "cancelled";

async function* frames(body: ReadableStream<Uint8Array>, signal: AbortSignal) {
  const reader = body.getReader();
  const cancelReader = () => { void reader.cancel().catch(() => {}); };
  const decoder = new TextDecoder("utf-8", { fatal: true });
  let buffer = "";
  let fields: string[] = [];
  let size = 0;
  signal.addEventListener("abort", cancelReader, { once: true });
  try {
    signal.throwIfAborted();
    for (;;) {
      const { value, done } = await reader.read();
      signal.throwIfAborted();
      buffer += decoder.decode(value, { stream: !done });
      let from = 0;
      // Preserve an ending CR until the next chunk distinguishes CR from CRLF.
      const endings = done ? /\r\n|\r|\n/g : /\r\n|\r(?!$)|\n/g;
      for (const separator of buffer.matchAll(endings)) {
        const at = separator.index;
        const line = buffer.slice(from, at);
        from = at + separator[0].length;
        if (!line) {
          if (fields.length) yield fields.join("\n");
          fields = [];
          size = 0;
          continue;
        }
        if (line.startsWith("data:")) {
          const field = line.slice(5).replace(/^ /, "");
          size += field.length + 1;
          if (size > frameLimit) throw new Error("聊天事件超过大小上限。");
          fields.push(field);
        }
      }
      buffer = buffer.slice(from);
      if (buffer.length > frameLimit) throw new Error("聊天事件超过大小上限。");
      if (done) break;
    }
  } finally {
    signal.removeEventListener("abort", cancelReader);
    cancelReader();
    reader.releaseLock();
  }
}

export async function readChatEvents(
  bundle: QueryClientBundle,
  runId: string,
  signal: AbortSignal,
  receive: (value: ChatEvent) => void,
): Promise<TerminalEvent | null> {
  const { data, response } = await bundle.api.raw.POST("/chat/runs/{runId}/events", {
    params: { path: { runId } },
    headers: { Accept: "text/event-stream" },
    parseAs: "stream",
    signal,
  });
  const eventStream = response.headers.get("content-type")?.split(";")[0]?.trim() === "text/event-stream";
  if (!data || !eventStream || signal.aborted) {
    void data?.cancel().catch(() => {});
    signal.throwIfAborted();
    throw new Error("聊天事件流无效。");
  }
  let lastSequence = 0;
  for await (const frame of frames(data, signal)) {
    const value = event.parse(JSON.parse(frame));
    if (value.run_id !== runId || value.seq <= lastSequence) continue;
    lastSequence = value.seq;
    receive(value);
    if (value.type === "completed" || value.type === "failed" || value.type === "cancelled") return value.type;
  }
  return null;
}
