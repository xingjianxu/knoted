import { PREVIEW_BYTES } from "./inspect";
/** Retains at most 50 MiB of input; browser prefetch and total memory are separate. */
export async function readPreviewBytes(response: Response, controller: AbortController): Promise<Uint8Array<ArrayBuffer>> {
    if (!response.body)
        throw new Error("预览响应没有正文，请重试或下载原文件。");
    const reader = response.body.getReader();
    const cancel = () => { void reader.cancel().catch(() => { }); };
    controller.signal.addEventListener("abort", cancel, { once: true });
    const chunks: Uint8Array[] = [];
    let total = 0;
    try {
        controller.signal.throwIfAborted();
        const advertised = response.headers.get("content-length");
        if (advertised && /^\d+$/.test(advertised) && Number(advertised) > PREVIEW_BYTES) {
            throw new Error("预览限 50 MiB，请下载原文件。");
        }
        while (true) {
            const { done, value } = await reader.read();
            controller.signal.throwIfAborted();
            if (done)
                break;
            if (value.byteLength > PREVIEW_BYTES - total)
                throw new Error("预览限 50 MiB，请下载原文件。");
            // Copy only admitted bytes; do not retain arbitrary oversized backing buffers.
            chunks.push(value.slice());
            total += value.byteLength;
        }
        const bytes = new Uint8Array(total);
        let offset = 0;
        for (const chunk of chunks) {
            bytes.set(chunk, offset);
            offset += chunk.byteLength;
        }
        return bytes;
    }
    catch (error) {
        cancel();
        controller.abort();
        throw error;
    }
    finally {
        chunks.length = 0;
        controller.signal.removeEventListener("abort", cancel);
        reader.releaseLock();
    }
}
