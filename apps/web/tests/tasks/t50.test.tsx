import { readFileSync } from "node:fs";
import { StrictMode } from "react";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ApiRequestError, createApiClient } from "@knoted/api-client";
import { UiProvider } from "../../src/lib/ui-provider";
import { FilePreview, type FilePreviewProps } from "../../src/features/files/preview/FilePreview";
import { inspectPreview, type FileSource } from "../../src/features/files/preview/inspect";
import { readPreviewBytes } from "../../src/features/files/preview/read";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { FileDetail } from "../../src/features/files/FileDetail";
import { queryKeys, type QueryClientBundle } from "../../src/lib/query";
import { createAppRouter } from "../../src/lib/router";
import { createMemoryHistory } from "@tanstack/react-router";
import App from "../../src/App";
const enc = new TextEncoder();
const source = (name = "a.txt", patch: Partial<FileSource> = {}): FileSource => ({ id: "a", name, relative_path: name, size: 12, mime: null, sha256: "a".repeat(64), version: 7, content_version: 3, status: "present", note: null, tag_ids: [], created_at: "2026-11-18T00:00:00Z", updated_at: "2026-11-18T00:00:00Z", last_seen_at: null, ...patch });
const fixture = (name: string) => new Uint8Array(readFileSync(`${process.cwd()}/tests/fixtures/t50/${name}`));
const concat = (...parts: Uint8Array[]) => { const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0)); let at = 0; for (const p of parts) {
    out.set(p, at);
    at += p.length;
} return out; };
const u32 = (n: number, le = false) => { const b = new Uint8Array(4); new DataView(b.buffer).setUint32(0, n, le); return b; };
const pngChunk = (type: string, bytes = new Uint8Array()) => concat(u32(bytes.length), enc.encode(type), bytes, new Uint8Array(4));
function png(w: number, h: number, extra = new Uint8Array()) { return concat(new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10]), pngChunk("IHDR", concat(u32(w), u32(h), new Uint8Array([8, 2, 0, 0, 0]))), pngChunk("IDAT", new Uint8Array([1])), extra, pngChunk("IEND")); }
const segment = (marker: number, payload: number[]) => new Uint8Array([255, marker, (payload.length + 2) >> 8, (payload.length + 2) & 255, ...payload]);
const sof = (w: number, h: number, marker = 0xc0) => segment(marker, [8, h >> 8, h & 255, w >> 8, w & 255, 1, 1, 0x11, 0]);
const sos = segment(0xda, [1, 1, 0, 0, 63, 0]);
function jpeg(w: number, h: number, late = new Uint8Array(), marker = 0xc0) { return concat(new Uint8Array([255, 216]), sof(w, h, marker), sos, new Uint8Array([1, 255, 0, 3, 255, 208, 4]), late, new Uint8Array([255, 217])); }
const webpChunk = (type: string, body: Uint8Array) => concat(enc.encode(type), u32(body.length, true), body, new Uint8Array(body.length % 2));
const riff = (...chunks: Uint8Array[]) => { const body = concat(enc.encode("WEBP"), ...chunks); return concat(enc.encode("RIFF"), u32(body.length, true), body); };
const vp8l = (w: number, h: number) => webpChunk("VP8L", concat(new Uint8Array([0x2f]), u32((w - 1) | ((h - 1) << 14), true), new Uint8Array([0])));
const vp8 = (w: number, h: number) => webpChunk("VP8 ", new Uint8Array([0x10, 0, 0, 0x9d, 1, 0x2a, w & 255, w >> 8, h & 255, h >> 8, 0]));
const vp8x = (w: number, h: number, flags = 0) => webpChunk("VP8X", new Uint8Array([flags, 0, 0, 0, (w - 1) & 255, ((w - 1) >> 8) & 255, ((w - 1) >> 16) & 255, (h - 1) & 255, ((h - 1) >> 8) & 255, ((h - 1) >> 16) & 255]));
// Mutations caught: permissive decoding, double BOM removal, execution via HTML sinks.
describe("strict text", () => {
    it.each([
        ["UTF8", enc.encode("中文\r\n<script>x</script>"), "中文\r\n<script>x</script>"],
        ["UTF8 BOM", new Uint8Array([239, 187, 191, 65]), "A"],
        ["double UTF8 BOM", new Uint8Array([239, 187, 191, 239, 187, 191, 65]), "\ufeffA"],
        ["UTF16 LE", new Uint8Array([255, 254, 45, 78]), "中"],
        ["UTF16 BE", new Uint8Array([254, 255, 78, 45]), "中"],
        ["double UTF16 BOM", new Uint8Array([255, 254, 255, 254, 65, 0]), "\ufeffA"],
        ["empty", new Uint8Array(), ""],
    ])("preserves %s", (_, bytes, expected) => expect(inspectPreview(source(), bytes as Uint8Array)).toEqual({ kind: "text", text: expected }));
    it.each([[0xff], [0xc0, 0xaf], [0xff, 0xfe, 1], [0xff, 0xfe, 0, 0], [0, 0, 0xfe, 0xff], [0xff, 0xfe, 0, 0xd8], [65, 0], [0xd6, 0xd0]])("rejects invalid encoding %j", (...bytes) => expect(() => inspectPreview(source(), new Uint8Array(bytes))).toThrow());
    it.each(["README", "LICENSE", "Makefile", "Dockerfile", "Justfile", "x.HTML", "x.md", "x.xml", "x.tsx", "x.toml"])("allows T46 text %s", name => expect(inspectPreview(source(name), enc.encode("text")).kind).toBe("text"));
    it.each(["x.svg", "x.gif", "x.avif", "x.docx", "x.exe", "unknown", "x.bin"])("rejects unsupported %s", name => expect(() => inspectPreview(source(name), enc.encode("text"))).toThrow());
});
describe("image containers before decoding", () => {
    it.each(["chinese.png", "chinese.jpg", "chinese.webp"])("accepts real static %s", name => expect(inspectPreview(source(name), fixture(name))).toEqual({ kind: "image", mime: name.endsWith("jpg") ? "image/jpeg" : `image/${name.split(".")[1]}` }));
    it.each(["chinese.png", "chinese.jpg", "chinese.webp"])("rejects truncated %s", name => expect(() => inspectPreview(source(name), fixture(name).slice(0, 25))).toThrow());
    it.each([["x.png", png(8000, 5000)], ["x.jpg", jpeg(8000, 5000)], ["x.webp", riff(vp8l(8000, 5000))]])("permits exactly 40M pixels in %s", (name, bytes) => expect(inspectPreview(source(name as string), bytes as Uint8Array).kind).toBe("image"));
    it.each([["x.png", png(40000001, 1)], ["x.jpg", jpeg(8001, 5000)], ["x.webp", riff(vp8l(8001, 5000))], ["x.png", png(0, 1)], ["x.jpg", jpeg(1, 0)]])("rejects unsafe dimensions %s", (name, bytes) => expect(() => inspectPreview(source(name as string), bytes as Uint8Array)).toThrow());
    it.each([png(1, 1, pngChunk("acTL")), png(1, 1, pngChunk("fcTL")), png(1, 1, pngChunk("fdAT")), png(1, 1, pngChunk("IHDR", new Uint8Array(13))), concat(png(1, 1), new Uint8Array([1])), png(1, 1).slice(0, -12)])("rejects PNG animation/duplicate/trailing/missing end %#", bytes => expect(() => inspectPreview(source("x.png"), bytes)).toThrow());
    it.each([jpeg(1, 1, sof(9000, 9000)), jpeg(1, 1, sof(1, 1)), jpeg(1, 1, segment(0xdc, [1, 1])), jpeg(1, 1, new Uint8Array([255, 0xe1, 255, 255])), jpeg(1, 1, new Uint8Array(), 0xc1)])("rejects JPEG late SOF/DNL/invalid segment/frame %#", bytes => expect(() => inspectPreview(source("x.jpg"), bytes)).toThrow());
    it("accepts progressive JPEG with metadata before frame and multiple scans", () => { const body = jpeg(2, 3, sos, 0xc2); const bytes = concat(body.slice(0, 2), segment(0xe1, Array(600).fill(0)), body.slice(2)); expect(inspectPreview(source("x.jpg"), bytes).kind).toBe("image"); });
    it.each([riff(vp8(3, 2)), riff(vp8l(3, 2)), riff(vp8x(3, 2), vp8(3, 2)), riff(vp8x(3, 2), vp8l(3, 2))])("accepts static WebP encodings %#", bytes => expect(inspectPreview(source("x.webp"), bytes).kind).toBe("image"));
    it.each([riff(vp8x(1, 1), vp8l(9000, 9000)), riff(vp8x(1, 1, 2), vp8l(1, 1)), riff(vp8l(1, 1), webpChunk("ANMF", new Uint8Array(16))), riff(vp8l(1, 1), vp8l(1, 1)), riff(vp8x(1, 1)), concat(riff(vp8l(1, 1)), new Uint8Array([1]))])("rejects WebP canvas/animation/duplicate/bounds %#", bytes => expect(() => inspectPreview(source("x.webp"), bytes)).toThrow());
    it("does not route mismatched binary signatures or MIME into active renderers", () => { expect(() => inspectPreview(source("x.png"), enc.encode("<html>"))).toThrow(); expect(() => inspectPreview(source("x.pdf"), fixture("chinese.png"))).toThrow(); expect(() => inspectPreview(source("x.png", { mime: "image/svg+xml" }), fixture("chinese.png"))).toThrow(); });
});
function response(bytes: Uint8Array, headers: Record<string, string> = {}) { return new Response(bytes as Uint8Array<ArrayBuffer>, { headers: { etag: '"7"', ...headers } }); }
function streaming(chunks: Uint8Array[], headers: Record<string, string> = {}) { const cancel = vi.fn(); let i = 0; const body = new ReadableStream<Uint8Array>({ pull(c) { if (i < chunks.length)
        c.enqueue(chunks[i++]!);
    else
        c.close(); }, cancel }); return { response: new Response(body, { headers }), cancel }; }
describe("actual stream byte limit", () => {
    it("accepts empty and exactly 52,428,800 bytes without whole-response readers", async () => { const chunk = new Uint8Array(1024 * 1024); const r = streaming(Array(50).fill(chunk)).response; const blob = vi.spyOn(r, "blob"), array = vi.spyOn(r, "arrayBuffer"); expect((await readPreviewBytes(r, new AbortController())).length).toBe(52428800); expect(blob).not.toHaveBeenCalled(); expect(array).not.toHaveBeenCalled(); expect((await readPreviewBytes(response(new Uint8Array()), new AbortController())).length).toBe(0); });
    it.each<Record<string, string>>([{}, { "content-length": "1" }])("rejects actual cap+1 with missing/false length %j", async (headers) => { const chunk = new Uint8Array(1024 * 1024); const h = streaming([...Array(50).fill(chunk), new Uint8Array(1), new Uint8Array(1)], headers); const owner = new AbortController(); await expect(readPreviewBytes(h.response, owner)).rejects.toThrow(/50 MiB/); expect(h.cancel).toHaveBeenCalled(); expect(owner.signal.aborted).toBe(true); });
    it("cancels a single oversized chunk", async () => { const h = streaming([new Uint8Array(52428801), new Uint8Array(1)]); await expect(readPreviewBytes(h.response, new AbortController())).rejects.toThrow(/50 MiB/); expect(h.cancel).toHaveBeenCalled(); });
    it("rejects advertised oversize without retaining data", async () => { const h = streaming([new Uint8Array(1)], { "content-length": "52428801" }); const owner = new AbortController(); await expect(readPreviewBytes(h.response, owner)).rejects.toThrow(/50 MiB/); expect(h.cancel).toHaveBeenCalled(); expect(owner.signal.aborted).toBe(true); });
    it("rejects absent body and read failures", async () => { await expect(readPreviewBytes(new Response(null), new AbortController())).rejects.toThrow(); const r = new Response(new ReadableStream({ pull(c) { c.error(new Error("read failed")); } })); await expect(readPreviewBytes(r, new AbortController())).rejects.toThrow(); });
    it("cancels pending reader immediately on abort", async () => { const cancel = vi.fn(); const owner = new AbortController(); const promise = readPreviewBytes(new Response(new ReadableStream({ cancel })), owner); owner.abort(); await expect(promise).rejects.toThrow(); expect(cancel).toHaveBeenCalled(); });
});
const live = new Set<string>();
let serial = 0;
beforeEach(() => { vi.stubGlobal("URL", URL); URL.createObjectURL = vi.fn(() => { const url = `blob:t50-${++serial}`; live.add(url); return url; }); URL.revokeObjectURL = vi.fn(url => live.delete(url)); });
afterEach(() => { cleanup(); expect(live.size).toBe(0); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
function harness(fetch: typeof globalThis.fetch, file = source()) { const onUnauthorized = vi.fn(); const props: FilePreviewProps = { api: createApiClient({ baseUrl: "http://localhost/api/v1", fetch, onUnauthorized }), file, open: true, onOpenChange: vi.fn(), onDownload: vi.fn() }; const view = (patch: Partial<FilePreviewProps> = {}) => <UiProvider><FilePreview {...props} {...patch}/></UiProvider>; const mounted = render(view()); return { ...mounted, props, onUnauthorized, change: (patch: Partial<FilePreviewProps>) => mounted.rerender(view(patch)) }; }
function deferred<T>() { let resolve!: (v: T) => void; const promise = new Promise<T>(r => { resolve = r; }); return { promise, resolve }; }
it("t50_1 HTML remains escaped original text with no active nodes", async () => { const html = '<script>window.pwned=1</script><img src=x onerror="alert(1)"><a href="javascript:alert(1)">x</a>'; harness(async () => response(enc.encode(html)), source("a.html")); const text = await screen.findByLabelText("文件文本"); expect(text.textContent).toBe(html); expect(text.querySelector("script,img,a")).toBeNull(); expect(document.querySelector("iframe")).toBeNull(); });
it("t50_2 close ignores late fetch even when transport ignores abort and cancels its body", async () => { const d = deferred<Response>(); const h = harness(async () => d.promise, source("a.png")); h.change({ open: false }); const r = streaming([fixture("chinese.png")]); await act(async () => d.resolve(r.response)); expect(URL.createObjectURL).not.toHaveBeenCalled(); expect(r.cancel).toHaveBeenCalled(); expect(screen.queryByRole("dialog")).toBeNull(); });
it("t50_3 switch A→B→A cannot publish old A or overwrite current success", async () => { const d = deferred<Response>(); let n = 0; const h = harness(async () => ++n === 1 ? d.promise : response(enc.encode(n === 2 ? "second" : "fresh A"))); h.change({ file: source("b.txt", { id: "b" }) }); expect(await screen.findByLabelText("文件文本")).toHaveTextContent("second"); h.change({ file: source() }); expect(screen.queryByText("second")).toBeNull(); expect(await screen.findByLabelText("文件文本")).toHaveTextContent("fresh A"); await act(async () => d.resolve(response(enc.encode("old A")))); expect(screen.getByLabelText("文件文本")).toHaveTextContent("fresh A"); });
it("revokes each loaded image URL on switch, close and unmount", async () => { const h = harness(async () => response(fixture("chinese.png")), source("a.png")); const first = (await screen.findByRole("img")).getAttribute("src"); expect(live.size).toBe(1); h.change({ file: source("b.png", { id: "b" }) }); await waitFor(() => expect(screen.getByRole("img").getAttribute("src")).not.toBe(first)); expect(live.size).toBe(1); expect(URL.revokeObjectURL).toHaveBeenCalledWith(first); h.change({ open: false }); expect(live.size).toBe(0); h.change({ open: true }); await screen.findByRole("img"); h.unmount(); expect(live.size).toBe(0); });
it("StrictMode discarded generation never publishes or leaks", async () => { const api = createApiClient({ baseUrl: "http://localhost/api/v1", fetch: async () => response(fixture("chinese.png")) }); const h = render(<StrictMode><UiProvider><FilePreview api={api} file={source("a.png")} open onOpenChange={() => { }} onDownload={() => { }}/></UiProvider></StrictMode>); await screen.findByRole("img"); expect(live.size).toBe(1); h.unmount(); expect(live.size).toBe(0); });
it("rejects oversized images before any object URL or img src and preserves download", async () => { const h = harness(async () => response(png(40000001, 1)), source("a.png")); await screen.findByText(/40,000,000/); expect(URL.createObjectURL).not.toHaveBeenCalled(); expect(screen.queryByRole("img")).toBeNull(); fireEvent.click(screen.getByRole("button", { name: "下载原文件" })); expect(h.props.onDownload).toHaveBeenCalledWith(h.props.file); });
it("metadata ETag mismatch asks refresh without claiming content changed", async () => { harness(async () => response(enc.encode("secret"), { etag: '"8"' })); await screen.findByText(/元数据版本/); expect(screen.queryByLabelText("文件文本")).toBeNull(); expect(screen.queryByText(/正文.*变化/)).toBeNull(); });
it("401 uses existing client handling and clears the dialog", async () => { const h = harness(async () => new Response(JSON.stringify({ code: "unauthorized", message: "unauthorized", request_id: "x" }), { status: 401, headers: { "content-type": "application/json" } })); await waitFor(() => expect(h.onUnauthorized).toHaveBeenCalledOnce()); await waitFor(() => expect(h.props.onOpenChange).toHaveBeenCalledWith(false)); expect(screen.queryByLabelText("文件文本")).toBeNull(); expect(live.size).toBe(0); });
it("DTO oversize skips preview fetching but ordinary download remains usable", async () => { const fetch = vi.fn(async () => response(enc.encode("unused"))); const h = harness(fetch, source("a.txt", { size: 52428801 })); await screen.findByText(/50 MiB/); expect(fetch).not.toHaveBeenCalled(); fireEvent.click(screen.getByRole("button", { name: "下载原文件" })); expect(h.props.onDownload).toHaveBeenCalledOnce(); });
it("PDF iframe has title and fixed MIME, load never claims rendering success", async () => { harness(async () => response(enc.encode("%PDF-1.7\nnot validated pages")), source("a.pdf")); const frame = await screen.findByTitle("PDF 预览：a.pdf"); expect(frame).toHaveAttribute("src", expect.stringMatching(/^blob:/)); expect(URL.createObjectURL).toHaveBeenCalledWith(expect.objectContaining({ type: "application/pdf" })); fireEvent.load(frame); expect(screen.getByText(/无法显示.*下载/)).toBeInTheDocument(); expect(screen.queryByText(/渲染成功/)).toBeNull(); });
it("close during a pending stream cancels its reader and publishes nothing", async () => {
    const cancel = vi.fn();
    let streamController!: ReadableStreamDefaultController<Uint8Array>;
    const body = new ReadableStream<Uint8Array>({ start(c) { streamController = c; }, cancel });
    const h = harness(async () => new Response(body, { headers: { etag: '"7"' } }), source("a.png"));
    await act(async () => { streamController.enqueue(fixture("chinese.png")); });
    fireEvent.click(screen.getByRole("button", { name: "关闭预览" }));
    await waitFor(() => expect(cancel).toHaveBeenCalledOnce());
    expect(h.props.onOpenChange).toHaveBeenCalledWith(false);
    expect(URL.createObjectURL).not.toHaveBeenCalled();
});
it("image decode failure removes the URL and keeps ordinary download", async () => {
    harness(async () => response(fixture("chinese.png")), source("a.png"));
    fireEvent.error(await screen.findByRole("img"));
    await screen.findByText(/图片解码失败/);
    expect(live.size).toBe(0);
    expect(screen.queryByRole("img")).toBeNull();
    expect(screen.getByRole("button", { name: "下载原文件" })).toBeEnabled();
});
it("download 401 clears already loaded image and owns no remaining URLs", async () => {
    const h = harness(async () => response(fixture("chinese.png")), source("a.png"));
    await screen.findByRole("img");
    h.change({ onDownload: async () => { throw new ApiRequestError({ status: 401, code: "unauthorized", requestId: "x" }); } });
    fireEvent.click(screen.getByRole("button", { name: "下载原文件" }));
    await waitFor(() => expect(h.props.onOpenChange).toHaveBeenCalledWith(false));
    expect(live.size).toBe(0);
    expect(screen.queryByRole("img")).toBeNull();
});
it.each([
    { version: 8 }, { content_version: 4 }, { sha256: "b".repeat(64) }, { mime: "text/plain" },
])("same ID identity change clears old text immediately %j", async (patch) => {
    const next = deferred<Response>();
    let n = 0;
    const h = harness(async () => ++n === 1 ? response(enc.encode("old text")) : next.promise);
    await screen.findByLabelText("文件文本");
    h.change({ file: source("a.txt", patch) });
    expect(screen.queryByLabelText("文件文本")).toBeNull();
    await act(async () => next.resolve(response(enc.encode("new text"), { etag: `"${patch.version ?? 7}"` })));
    expect(await screen.findByLabelText("文件文本")).toHaveTextContent("new text");
});
it("late failed A response cannot replace successful B", async () => {
    const d = deferred<Response>();
    let n = 0;
    const h = harness(async () => ++n === 1 ? d.promise : response(enc.encode("B text")));
    h.change({ file: source("b.txt", { id: "b" }) });
    await screen.findByLabelText("文件文本");
    await act(async () => d.resolve(new Response(JSON.stringify({ code: "unavailable", message: "old error", request_id: "x" }), { status: 503, headers: { "content-type": "application/json" } })));
    expect(screen.getByLabelText("文件文本")).toHaveTextContent("B text");
    expect(screen.queryByText("old error")).toBeNull();
});
it("multibyte text spanning network chunks is decoded once without replacement", async () => {
    const r = streaming([new Uint8Array([0xe4]), new Uint8Array([0xb8]), new Uint8Array([0xad])], { etag: '"7"' });
    harness(async () => r.response);
    expect(await screen.findByLabelText("文件文本")).toHaveTextContent("中");
});
it.each([
    ["x.jpg", jpeg(1, 1, sof(9000, 9000))], ["x.jpg", jpeg(1, 1, segment(0xdc, [1, 1]))],
    ["x.png", png(1, 1, pngChunk("acTL"))], ["x.webp", riff(vp8x(1, 1), vp8l(9000, 9000))],
])("malicious containers never receive an image URL or src %s", async (name, bytes) => {
    harness(async () => response(bytes as Uint8Array), source(name as string));
    await screen.findByText(/请下载原文件/);
    expect(URL.createObjectURL).not.toHaveBeenCalled();
    expect(screen.queryByRole("img")).toBeNull();
});
it("actual streamed oversize retains the independent download action", async () => {
    const r = streaming([...Array(50).fill(new Uint8Array(1024 * 1024)), new Uint8Array(1), new Uint8Array(1)], { etag: '"7"', "content-length": "1" });
    const h = harness(async () => r.response);
    await screen.findByText(/50 MiB/);
    expect(r.cancel).toHaveBeenCalled();
    expect(URL.createObjectURL).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "下载原文件" }));
    expect(h.props.onDownload).toHaveBeenCalledOnce();
});
it("request uses the existing same-origin typed endpoint without new preview parameters", async () => {
    const requests: Request[] = [];
    harness(async (input) => { requests.push(input as Request); return response(enc.encode("ok")); }, source("a.txt", { id: "a/b" }));
    await screen.findByLabelText("文件文本");
    expect(requests[0]?.url).toBe("http://localhost/api/v1/files/a%2Fb/content");
    expect(requests[0]?.credentials).toBe("same-origin");
});
it("does not expand T46 text selection to extension-only hidden basenames", () => {
    expect(() => inspectPreview(source(".txt"), enc.encode("hidden"))).toThrow();
    expect(() => inspectPreview(source("x.KT"), enc.encode("unicode extension"))).toThrow();
});
it("closing between URL creation and publication revokes that late URL", async () => {
    const d = deferred<Response>();
    const h = harness(async () => d.promise, source("a.png"));
    const create = URL.createObjectURL;
    vi.mocked(URL.createObjectURL).mockImplementationOnce(blob => { const url = `blob:late-${++serial}`; live.add(url); h.unmount(); expect(blob).toBeInstanceOf(Blob); if (!(blob instanceof Blob)) throw new Error("Expected a preview Blob"); expect(blob.type).toBe("image/png"); return url; });
    await act(async () => d.resolve(response(fixture("chinese.png"))));
    expect(live.size).toBe(0);
    expect(screen.queryByRole("img")).toBeNull();
    expect(create).toHaveBeenCalledOnce();
});
it.each([
    ["x.png", concat(png(1, 1).slice(0, 33), u32(0xffffffff), enc.encode("IDAT"), new Uint8Array(4))],
    ["x.jpg", concat(new Uint8Array([255, 216]), sos, new Uint8Array([255, 217]))],
    ["x.jpg", concat(new Uint8Array([255, 216, 255, 225, 0, 1]), jpeg(1, 1).slice(2))],
    ["x.webp", concat(enc.encode("RIFF"), u32(16, true), enc.encode("WEBPVP8L"), u32(0xffffffff, true), new Uint8Array(4))],
    ["x.webp", riff(webpChunk("VP8 ", new Uint8Array([1, 0, 0, 0x9d, 1, 0x2a, 1, 0, 1, 0])))],
])("rejects invalid container boundaries and unknown dimensions %s", (name, bytes) => expect(() => inspectPreview(source(name as string), bytes as Uint8Array)).toThrow());

// Phase B exercises the actual FileDetail callsite; A's 108 tests are retained.
const fileA = "00000000-0000-4000-8000-000000000050";
const fileB = "00000000-0000-4000-8000-000000000051";
const jsonResponse = (value: unknown, status = 200) => new Response(JSON.stringify(value), {status, headers:{"content-type":"application/json"}});
function detailHarness(file = source("预览原件.txt", {id:fileA, note:"saved note", tag_ids:[fileB]}), content: () => Response | Promise<Response> = () => response(enc.encode("actual source text"))) {
    const requests: Request[] = [];
    const bundle: QueryClientBundle = {
        queryClient: new QueryClient({defaultOptions:{queries:{retry:false,staleTime:Infinity}}}),
        api: createApiClient({baseUrl:"http://localhost/api/v1",fetch:async input=>{
            const request = input as Request; requests.push(request);
            if (request.url.endsWith("/content")) return content();
            if (new URL(request.url).pathname.endsWith("/tags")) return jsonResponse({items:[],next_cursor:null});
            return jsonResponse(request.url.endsWith(fileB) ? source("second.txt",{id:fileB}) : file);
        }}),
    };
    const view=(id=file.id)=><UiProvider><QueryClientProvider client={bundle.queryClient}><FileDetail bundle={bundle} fileId={id} onSelectFile={()=>{}}/></QueryClientProvider></UiProvider>;
    const mounted=render(view());
    return {...mounted,bundle,requests,select:(id:string)=>mounted.rerender(view(id))};
}
async function openDetailPreview() {
    const opener = await screen.findByRole("button",{name:"预览文件"});
    opener.focus(); fireEvent.click(opener);
    return {opener,dialog:await screen.findByRole("dialog")};
}
it.each(["button","Escape"])("B real detail preview closes by %s, restores opener and retains dirty note/tags",async(method)=>{
    const h=detailHarness();
    fireEvent.change(await screen.findByLabelText("文件备注"),{target:{value:"unsaved private draft"}});
    fireEvent.click(screen.getByRole("button",{name:new RegExp(`^(?:移除|Remove) ${fileB}$`)}));
    const {opener,dialog}=await openDetailPreview();
    expect(await within(dialog).findByLabelText("文件文本")).toHaveTextContent("actual source text");
    if(method==="Escape") fireEvent.keyDown(within(dialog).getByRole("button",{name:"关闭预览"}),{key:"Escape",code:"Escape"});
    else fireEvent.click(within(dialog).getByRole("button",{name:"关闭预览"}));
    await waitFor(()=>expect(screen.queryByRole("dialog")).toBeNull());
    await waitFor(()=>expect(opener).toHaveFocus());
    expect(screen.getByLabelText("文件备注")).toHaveValue("unsaved private draft");
    expect(screen.queryByRole("button",{name:new RegExp(`^(?:移除|Remove) ${fileB}$`)})).toBeNull();
    expect(screen.getByRole("button",{name:"保存备注和标签"})).toBeEnabled();
    expect(h.requests.filter(r=>r.method!=="GET")).toHaveLength(0);
    await openDetailPreview(); expect(await screen.findByLabelText("文件文本")).toHaveTextContent("actual source text");
});
it("B missing sources expose a disabled preview action and never request content",async()=>{
    const h=detailHarness(source("missing.txt",{id:fileA,status:"missing"}));
    expect(await screen.findByRole("button",{name:"预览文件"})).toBeDisabled();
    expect(h.requests.some(r=>r.url.endsWith("/content"))).toBe(false);
});
it("B ordinary download uses the opened snapshot and original bytes after detail metadata refresh",async()=>{
    const original = new Uint8Array([255,254,45,78]);
    const downloads:string[]=[];
    vi.spyOn(HTMLAnchorElement.prototype,"click").mockImplementation(function(this: HTMLAnchorElement){downloads.push(this.download);});
    const h=detailHarness(source("原件.txt",{id:fileA}),()=>response(original));
    const {dialog}=await openDetailPreview();
    expect(await within(dialog).findByLabelText("文件文本")).toHaveTextContent("中");
    act(()=>h.bundle.queryClient.setQueryData(queryKeys.files.detail(fileA),source("新的名称.txt",{id:fileA,version:8})));
    fireEvent.click(within(dialog).getByRole("button",{name:"下载原文件"}));
    await waitFor(()=>expect(downloads).toEqual(["原件.txt"]));
    const blob=vi.mocked(URL.createObjectURL).mock.calls.at(-1)?.[0];
    expect(blob).toHaveProperty("size",4);
    expect(Array.from(new Uint8Array(await (blob as Blob).arrayBuffer()))).toEqual([255,254,45,78]);
    expect(h.requests.filter(r=>r.url.endsWith("/content"))).toHaveLength(2);
    expect(live.size).toBe(0);
});
it("B over-limit preview retains the same ordinary download path without the preview cap",async()=>{
    const downloads:string[]=[];
    vi.spyOn(HTMLAnchorElement.prototype,"click").mockImplementation(function(this: HTMLAnchorElement){downloads.push(this.download);});
    const h=detailHarness(source("large.txt",{id:fileA,size:52428801}),()=>response(new Uint8Array(52428801)));
    const {dialog}=await openDetailPreview(); await within(dialog).findByText(/50 MiB/);
    expect(h.requests.filter(r=>r.url.endsWith("/content"))).toHaveLength(0);
    fireEvent.click(within(dialog).getByRole("button",{name:"下载原文件"}));
    await waitFor(()=>expect(downloads).toEqual(["large.txt"]));
    expect(vi.mocked(URL.createObjectURL).mock.calls.at(-1)?.[0]).toHaveProperty("size",52428801);
    expect(h.requests.filter(r=>r.url.endsWith("/content"))).toHaveLength(1);
    expect(live.size).toBe(0);
});
it("B switching the real detail removes the active preview and revokes its image",async()=>{
    const h=detailHarness(source("a.png",{id:fileA}),()=>response(fixture("chinese.png")));
    await openDetailPreview(); await screen.findByRole("img"); expect(live.size).toBe(1);
    h.select(fileB); await screen.findByRole("button",{name:"预览文件"});
    expect(screen.queryByRole("dialog")).toBeNull(); expect(live.size).toBe(0);
});
it("B a failed ordinary download is visible inside the preview",async()=>{
    let calls=0;
    detailHarness(undefined,()=>++calls===1?response(enc.encode("text")):jsonResponse({code:"dependency_unavailable",message:"offline",request_id:fileA},503));
    const {dialog}=await openDetailPreview();await within(dialog).findByLabelText("文件文本");
    fireEvent.click(within(dialog).getByRole("button",{name:"下载原文件"}));
    expect(await within(dialog).findByText("下载失败，请重试。")).toBeInTheDocument();
});

it.each(["preview401","loaded-detail401"] as const)("B default router %s removes preview resources, draft and file cache",async(mode)=>{
    const {createServer}=await import("node:http");
    let expired=false;
    const file=source("session.png",{id:fileA,note:"saved note"});
    const server=createServer((request,res)=>{
        const path=new URL(request.url!,"http://localhost").pathname;
        const reply=(value:unknown,status=200)=>{res.writeHead(status,{"content-type":"application/json"});res.end(JSON.stringify(value));};
        if(path==="/api/v1/auth/session") return reply({session:{id:fileA,user_id:fileB,current:true},csrf_token:"test-csrf-value-long"});
        if((mode==="preview401"&&path.endsWith("/content"))||(expired&&path===`/api/v1/files/${fileA}`)) return reply({code:"unauthorized",message:"expired",request_id:fileA},401);
        if(path.endsWith("/content")){res.writeHead(200,{"content-type":"application/octet-stream",etag:'"7"'});res.end(fixture("chinese.png"));return;}
        if(path==="/api/v1/files") return reply({items:[file],next_cursor:null});
        if(path===`/api/v1/files/${fileA}`)return reply(file);
        if(path==="/api/v1/tags")return reply({items:[],next_cursor:null});
        reply({},404);
    });
    await new Promise<void>(resolve=>server.listen(0,"127.0.0.1",resolve));
    const address=server.address() as import("node:net").AddressInfo;
    const NativeRequest=globalThis.Request;
    class OriginRequest extends NativeRequest {constructor(input:RequestInfo|URL,init?:RequestInit){super(typeof input==="string"?new URL(input,`http://127.0.0.1:${address.port}`):input,init);}}
    vi.stubGlobal("Request",OriginRequest);
    const router=createAppRouter({history:createMemoryHistory({initialEntries:[`/files/${fileA}`]})});
    const bundle=router.options.context!;
    const mounted=render(<UiProvider><App router={router}/></UiProvider>);
    try{
        await act(async()=>router.load());
        fireEvent.change(await screen.findByLabelText("文件备注"),{target:{value:"private preview session draft"}});
        fireEvent.click(await screen.findByRole("button",{name:"预览文件"}));
        if(mode==="loaded-detail401"){
            await screen.findByRole("img");expect(live.size).toBe(1);expired=true;
            await act(async()=>bundle.queryClient.invalidateQueries({queryKey:queryKeys.files.detail(fileA)}));
        }
        await waitFor(()=>expect(router.state.location.pathname).toBe("/login"));
        expect(router.state.location.search).toMatchObject({redirect:`/files/${fileA}`});
        expect(screen.queryByRole("dialog")).toBeNull();expect(screen.queryByDisplayValue("private preview session draft")).toBeNull();
        expect(bundle.queryClient.getQueryData(queryKeys.files.detail(fileA))).toBeUndefined();expect(live.size).toBe(0);
    }finally{
        mounted.unmount();await bundle.queryClient.cancelQueries();bundle.queryClient.clear();vi.stubGlobal("Request",NativeRequest);
        server.closeAllConnections();await new Promise<void>((resolve,reject)=>server.close(error=>error?reject(error):resolve()));
    }
});
