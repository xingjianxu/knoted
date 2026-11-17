import { File as NodeFile } from "node:buffer";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useBlockNoteEditor } from "@blocknote/react";
import { useEffect } from "react";
import { beforeAll, afterAll, afterEach, expect, it, vi } from "vitest";
import { NoteEditor, noteEditorSchema, type NoteDocument, type NoteEditorInstance } from "../../src/features/notes/editor";
import { createApiClient } from "@knoted/api-client";
import { ImageBlockControls, CodeBlockControls } from "../../src/features/notes/editor/blocks";
import { UiProvider } from "../../src/lib/ui-provider";

const assetId = "12345678-1234-4234-8234-123456789abc";
const initial: NoteDocument = { blocks: [
  { id: "image", type: "image", asset_id: assetId, content: [{ type: "text", text: "caption" }] },
  { id: "code", type: "codeBlock", language: "custom <dialect>", content: [{ type: "text", text: '<img src=x onerror="boom()"> & <script>alert(1)</script>', marks: ["bold", "code"] }] },
  { id: "p", type: "paragraph", content: [{ type: "text", text: "next" }] },
] };
beforeAll(() => {
  vi.spyOn(window, "scrollBy").mockImplementation(() => {});
  Object.defineProperty(Range.prototype, "getClientRects", { configurable: true, value: () => [] });
  Object.defineProperty(Range.prototype, "getBoundingClientRect", { configurable: true, value: () => new DOMRect() });
});
afterAll(() => {
  vi.restoreAllMocks();
  Reflect.deleteProperty(Range.prototype, "getClientRects");
  Reflect.deleteProperty(Range.prototype, "getBoundingClientRect");
});
class BrowserRequest extends Request {
  constructor(input: RequestInfo | URL, init?: RequestInit) {
    super(typeof input === "string" ? new URL(input, window.location.origin) : input, init);
  }
}
afterEach(() => { vi.unstubAllGlobals(); document.cookie = "knoted_csrf=; Max-Age=0; path=/"; });
const asset = { id: "87654321-1234-4234-8234-123456789abc", mime: "image/png", size: 68, sha256: "a".repeat(64), state: "ready", created_at: "2026-11-17T00:00:00Z" };
function response() { return new Response(JSON.stringify(asset), { status: 201, headers: { "Content-Type": "application/json" } }); }
function deferred() { let resolve!: (value: Response) => void; const promise = new Promise<Response>((r) => { resolve = r; }); return { promise, resolve }; }
async function setup(document = initial, fetcher: typeof fetch = async () => response(), blockId?: string) {
  vi.stubGlobal("Request", BrowserRequest);
  // The Request transport is Node's real fetch implementation. Its multipart
  // encoder requires Node File/FormData rather than jsdom's DOM-only objects.
  const nativeForm = await new BrowserRequest("/", { method: "POST", headers: { "Content-Type": "application/x-www-form-urlencoded" }, body: "" }).formData();
  vi.stubGlobal("FormData", nativeForm.constructor);
  vi.stubGlobal("File", NodeFile);
  const client = createApiClient({ baseUrl: "/api/v1", fetch: fetcher });
  let editor!: NoteEditorInstance;
  function Probe() {
    const value = useBlockNoteEditor(noteEditorSchema);
    useEffect(() => { editor = value; }, [value]);
    return null;
  }
  const onChange = vi.fn();
  const view = render(<UiProvider><NoteEditor document={document} onChange={onChange}><ImageBlockControls client={client} blockId={blockId} /><CodeBlockControls /><Probe /></NoteEditor></UiProvider>);
  await waitFor(() => expect(editor).toBeDefined());
  return { ...view, editor, onChange, client };
}
it("t29_1: reopening persisted asset_id renders authenticated same-origin content and retains the caption", async () => {
  await setup();
  expect(screen.getByRole("img", { name: "笔记图片" })).toHaveAttribute("src", `/api/v1/assets/${assetId}/content`);
  expect(screen.getByText("caption")).toBeInTheDocument();
});
it("t29_2: code special characters remain text, with marks and custom language preserved", async () => {
  const { container } = await setup();
  expect(container.querySelector("pre code")?.textContent).toBe('<img src=x onerror="boom()"> & <script>alert(1)</script>');
  expect(container.querySelector("pre img, pre script")).toBeNull();
  expect(container.querySelector("pre")).toHaveAttribute("data-language", "custom <dialect>");
});

function chooseFile() {
  const file = new File(["public PNG fixture"], "pixel.png", { type: "image/png" });
  fireEvent.change(screen.getByLabelText("选择文件"), { target: { files: [file] } });
}
it("t29_2: language control represents loaded custom values and preserves plain text/marks through change and undo", async () => {
  const { editor, onChange } = await setup();
  act(() => editor.setTextCursorPosition("code", "end"));
  const select = await screen.findByRole("combobox", { name: "代码语言" });
  expect(select).toHaveTextContent("custom <dialect>");
  fireEvent.click(select);
  const option = await screen.findByRole("option", { name: "JavaScript" });
  fireEvent.pointerDown(option, { pointerType: "mouse" });
  fireEvent.click(option);
  await waitFor(() => expect(onChange).toHaveBeenCalledTimes(1));
  expect(onChange.mock.lastCall![0].blocks[1]).toEqual({ ...initial.blocks[1], language: "javascript" });
  act(() => editor.undo());
  expect(editor.getBlock("code")!.props.language).toBe("custom <dialect>");
});
it("t29_3: failed upload keeps old image, retries same logical idempotency key with typed multipart/CSRF/cookies, then one undo restores it", async () => {
  const requests: Request[] = [];
  const fetcher: typeof fetch = async (input) => {
    requests.push(input as Request);
    return requests.length === 1 ? new Response(JSON.stringify({ code: "SERVICE_UNAVAILABLE", message: "retry fixture" }), { status: 503 }) : response();
  };
  const { editor, onChange } = await setup(initial, fetcher, "image");
  document.cookie = "knoted_csrf=public-fixture-csrf; path=/";
  chooseFile();
  await screen.findByRole("button", { name: "重试" });
  expect(onChange).not.toHaveBeenCalled();
  expect(editor.getBlock("image")!.props.assetId).toBe(assetId);
  fireEvent.click(screen.getByRole("button", { name: "重试" }));
  await screen.findByText("上传成功");
  expect(onChange).toHaveBeenCalledTimes(1);
  expect(onChange.mock.lastCall![0].blocks[0]).toEqual({ ...initial.blocks[0], asset_id: asset.id });
  expect(requests).toHaveLength(2);
  for (const request of requests) {
    expect(request.url).toBe("http://localhost:3000/api/v1/assets");
    expect(request.method).toBe("POST");
    expect(request.credentials).toBe("same-origin");
    expect(request.headers.get("X-CSRF-Token")).toBe("public-fixture-csrf");
    expect(request.headers.get("Content-Type")).toMatch(/^multipart\/form-data; boundary=/);
    const body = await request.formData();
    expect(body.get("file")).toMatchObject({ name: "pixel.png", type: "image/png" });
  }
  expect(requests[0]!.headers.get("Idempotency-Key")).toBe(requests[1]!.headers.get("Idempotency-Key"));
  expect(requests[0]!.headers.get("Idempotency-Key")).toMatch(/^[0-9a-f-]{36}$/);
  act(() => editor.undo());
  expect(editor.getBlock("image")!.props.assetId).toBe(assetId);
});
it("upload inserts beside the captured block after selection moves, without persisting a URL or temporary block", async () => {
  const pending = deferred();
  const { editor, onChange } = await setup(initial, async () => pending.promise);
  act(() => editor.setTextCursorPosition("p", "end"));
  chooseFile();
  expect(onChange).not.toHaveBeenCalled();
  act(() => editor.setTextCursorPosition("code", "end"));
  await act(async () => { pending.resolve(response()); });
  await screen.findByText("上传成功");
  expect(onChange).toHaveBeenCalledTimes(1);
  const blocks = onChange.mock.lastCall![0].blocks;
  expect(blocks[3]).toMatchObject({ type: "image", asset_id: asset.id });
  expect(JSON.stringify(blocks)).not.toContain("/api/");
  act(() => editor.undo());
  expect(editor.document.map((b) => b.id)).toEqual(["image", "code", "p"]);
});
it.each(["cancel", "unmount", "delete", "convert", "switch"])("late upload after %s cannot mutate the old document or a new note", async (action) => {
  const pending = deferred();
  let signal!: AbortSignal;
  const { editor, onChange, unmount, rerender } = await setup(initial, async (input) => { signal = (input as Request).signal; return pending.promise; }, "image");
  chooseFile();
  await waitFor(() => expect(signal).toBeDefined());
  if (action === "cancel") fireEvent.click(screen.getByRole("button", { name: "取消" }));
  if (action === "unmount") unmount();
  if (action === "delete") act(() => editor.removeBlocks(["image"]));
  if (action === "convert") act(() => editor.updateBlock("image", { type: "paragraph" }));
  if (action === "switch") rerender(<UiProvider><NoteEditor key="another-note" document={{ blocks: [{ id: "fresh", type: "paragraph" }] }} onChange={onChange} /></UiProvider>);
  onChange.mockClear();
  await act(async () => { pending.resolve(response()); });
  expect(signal.aborted).toBe(true);
  expect(onChange).not.toHaveBeenCalled();
  if (action === "cancel") expect(editor.getBlock("image")!.props.assetId).toBe(assetId);
});
it("a newer file for the same captured image supersedes the older in-flight upload", async () => {
  const first = deferred(); const second = deferred(); const requests: Request[] = [];
  const { editor, onChange } = await setup(initial, async (input) => { requests.push(input as Request); return requests.length === 1 ? first.promise : second.promise; }, "image");
  chooseFile(); chooseFile();
  await waitFor(() => expect(requests).toHaveLength(2));
  await act(async () => first.resolve(response()));
  expect(onChange).not.toHaveBeenCalled();
  expect(requests[0]!.signal.aborted).toBe(true);
  await act(async () => second.resolve(response()));
  await waitFor(() => expect(onChange).toHaveBeenCalledTimes(1));
  expect(editor.getBlock("image")!.props.assetId).toBe(asset.id);
});
it("cancel then retry retains the same logical key and original target after selection changes", async () => {
  const first = deferred(); const requests: Request[] = [];
  const { editor, onChange } = await setup(initial, async (input) => { requests.push(input as Request); return requests.length === 1 ? first.promise : response(); });
  act(() => editor.setTextCursorPosition("p", "end"));
  chooseFile();
  await waitFor(() => expect(requests).toHaveLength(1));
  fireEvent.click(screen.getByRole("button", { name: "取消" }));
  act(() => editor.setTextCursorPosition("image", "end"));
  fireEvent.click(screen.getByRole("button", { name: "重试" }));
  await screen.findByText("上传成功");
  expect(requests[1]!.headers.get("Idempotency-Key")).toBe(requests[0]!.headers.get("Idempotency-Key"));
  expect(onChange.mock.lastCall![0].blocks[3]).toMatchObject({ type: "image", asset_id: asset.id });
  expect(editor.getBlock("image")!.props.assetId).toBe(assetId);
  onChange.mockClear();
  await act(async () => first.resolve(response()));
  expect(onChange).not.toHaveBeenCalled();
});
it("deleting and undoing the original target does not revive an earlier upload", async () => {
  const pending = deferred();
  const { editor, onChange } = await setup(initial, async () => pending.promise, "image");
  chooseFile();
  act(() => { editor.removeBlocks(["image"]); editor.undo(); });
  onChange.mockClear();
  await act(async () => pending.resolve(response()));
  expect(onChange).not.toHaveBeenCalled();
  expect(editor.getBlock("image")!.props.assetId).toBe(assetId);
});
it("an image inserted beside a depth-8 target stays at depth 8", async () => {
  let nested: NoteDocument["blocks"][number] = { id: "p", type: "paragraph" };
  for (let i = 7; i >= 1; i--) nested = { id: `n-${i}`, type: "paragraph", children: [nested] };
  const { editor, onChange } = await setup({ blocks: [nested] });
  act(() => editor.setTextCursorPosition("p", "end")); chooseFile();
  await screen.findByText("上传成功");
  expect(onChange).toHaveBeenCalledTimes(1);
  expect(editor.getParentBlock(editor.getNextBlock("p")!)!.id).toBe("n-7");
});
it("at 5000 blocks refuses a new image before upload while allowing an existing image replacement", async () => {
  const fetcher = vi.fn<typeof fetch>(async () => response());
  const blocks: NoteDocument["blocks"] = Array.from({ length: 4999 }, (_, i) => ({ id: i ? `p-${i}` : "p", type: "paragraph" }));
  blocks.push(initial.blocks[0]!);
  const { editor, onChange } = await setup({ blocks }, fetcher);
  act(() => editor.setTextCursorPosition("p", "end")); chooseFile();
  await screen.findByRole("button", { name: "重试" });
  expect(fetcher).not.toHaveBeenCalled(); expect(onChange).not.toHaveBeenCalled();
  act(() => editor.setTextCursorPosition("image", "end")); chooseFile();
  await screen.findByText("上传成功");
  expect(editor.document).toHaveLength(5000);
  expect(editor.getBlock("image")!.props.assetId).toBe(asset.id);
}, 15000);

it("client rerenders keep a failed upload's original target and idempotency key", async () => {
  const requests: Request[] = [];
  const fetcher: typeof fetch = async input => {
    requests.push(input as Request);
    return requests.length === 1 ? new Response(JSON.stringify({ code: "SERVICE_UNAVAILABLE", message: "retry fixture" }), { status: 503 }) : response();
  };
  const { editor, onChange, rerender } = await setup(initial, fetcher);
  act(() => editor.setTextCursorPosition("p", "end")); chooseFile();
  await screen.findByRole("button", { name: "重试" });
  act(() => editor.setTextCursorPosition("image", "end"));
  const newClient = createApiClient({ baseUrl: "/api/v1", fetch: fetcher });
  rerender(<UiProvider><NoteEditor document={initial} onChange={onChange}><ImageBlockControls client={newClient} /><CodeBlockControls /></NoteEditor></UiProvider>);
  fireEvent.click(screen.getByRole("button", { name: "重试" }));
  await screen.findByText("上传成功");
  expect(requests[1]!.headers.get("Idempotency-Key")).toBe(requests[0]!.headers.get("Idempotency-Key"));
  expect(onChange.mock.lastCall![0].blocks[3]).toMatchObject({ type: "image", asset_id: asset.id });
  expect(editor.getBlock("image")!.props.assetId).toBe(assetId);
});
it("retargeting explicit image controls discards the old file UI and aborts its upload", async () => {
  const pending = deferred(); let signal!: AbortSignal;
  const document: NoteDocument = { blocks: [...initial.blocks, { id: "other", type: "image", asset_id: assetId }] };
  const { editor, onChange, rerender, client } = await setup(document, async input => { signal = (input as Request).signal; return pending.promise; }, "image");
  chooseFile(); await waitFor(() => expect(signal).toBeDefined());
  rerender(<UiProvider><NoteEditor document={document} onChange={onChange}><ImageBlockControls client={client} blockId="other" /><CodeBlockControls /></NoteEditor></UiProvider>);
  expect(screen.queryByText("pixel.png")).not.toBeInTheDocument();
  expect(signal.aborted).toBe(true);
  await act(async () => pending.resolve(response()));
  expect(onChange).not.toHaveBeenCalled();
  expect(editor.getBlock("other")!.props.assetId).toBe(assetId);
});
