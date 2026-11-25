import { File as NodeFile } from "node:buffer";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterAll, afterEach, beforeAll, expect, it, vi } from "vitest";
import { createApiClient } from "@knoted/api-client";

import { NoteEditor, uploadNoteImage, type NoteDocument } from "../../src/features/notes/editor";
import { UiProvider } from "../../src/lib/ui-provider";

const assetId = "12345678-1234-4234-8234-123456789abc";
const initial: NoteDocument = {
  blocks: [
    { id: "image", type: "image", asset_id: assetId, content: [{ type: "text", text: "caption" }] },
    {
      id: "code",
      type: "codeBlock",
      language: "custom <dialect>",
      content: [{ type: "text", text: '<img src=x onerror="boom()"> & <script>alert(1)</script>', marks: ["bold", "code"] }],
    },
    { id: "p", type: "paragraph", content: [{ type: "text", text: "next" }] },
  ],
};

beforeAll(() => {
  vi.spyOn(window, "scrollBy").mockImplementation(() => {});
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(() => new DOMRect(10, 10, 500, 30));
  Object.defineProperty(Range.prototype, "getClientRects", { configurable: true, value: () => [] });
  Object.defineProperty(Range.prototype, "getBoundingClientRect", {
    configurable: true,
    value: () => new DOMRect(),
  });
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

afterEach(() => {
  vi.unstubAllGlobals();
  document.cookie = "knoted_csrf=; Max-Age=0; path=/";
});

const asset = {
  id: "87654321-1234-4234-8234-123456789abc",
  mime: "image/png",
  size: 68,
  sha256: "a".repeat(64),
  state: "ready",
  created_at: "2026-11-17T00:00:00Z",
};
function response() {
  return new Response(JSON.stringify(asset), { status: 201, headers: { "Content-Type": "application/json" } });
}
function deferred() {
  let resolve!: (value: Response) => void;
  const promise = new Promise<Response>((r) => { resolve = r; });
  return { promise, resolve };
}

async function stubMultipartGlobals() {
  vi.stubGlobal("Request", BrowserRequest);
  const nativeForm = await new BrowserRequest("/", {
    method: "POST",
    headers: { "Content-Type": "application/x-www-form-urlencoded" },
    body: "",
  }).formData();
  vi.stubGlobal("FormData", nativeForm.constructor);
  vi.stubGlobal("File", NodeFile);
}

async function setup(document = initial, fetcher: typeof fetch = async () => response()) {
  await stubMultipartGlobals();
  const client = createApiClient({ baseUrl: "/api/v1", fetch: fetcher });
  const onChange = vi.fn();
  const view = render(
    <UiProvider>
      <NoteEditor documentKey="t29" document={document} client={client} onChange={onChange} />
    </UiProvider>,
  );
  const editable = await waitFor(() => {
    const el = view.container.querySelector<HTMLElement>('[contenteditable="true"]');
    expect(el).not.toBeNull();
    return el!;
  });
  return { ...view, editable, onChange, client };
}

async function openImageEditForm() {
  const figure = screen.getByRole("img", { name: "笔记图片" }).closest("figure")!;
  fireEvent.click(figure);
  const editButton = await screen.findByText("编辑图片");
  fireEvent.mouseDown(editButton);
  fireEvent.click(editButton);
  return screen.findByRole("group", { name: "图片设置", hidden: true });
}

it("t29_1: reopening persisted asset_id renders authenticated same-origin content and retains the caption", async () => {
  await setup();
  expect(screen.getByRole("img", { name: "笔记图片" })).toHaveAttribute(
    "src",
    `${window.location.origin}/api/v1/assets/${assetId}/content`,
  );
  expect(screen.getByText("caption")).toBeInTheDocument();
});

it("t29_2: code special characters remain text, with custom language represented and editable via CodeControls", async () => {
  const { container, editable, onChange } = await setup({
    blocks: [initial.blocks[1]!, initial.blocks[2]!],
  });
  expect(container.querySelector("pre code")?.textContent).toBe('<img src=x onerror="boom()"> & <script>alert(1)</script>');
  expect(container.querySelector("pre img, pre script")).toBeNull();

  await act(async () => {
    editable.focus();
    fireEvent.focusIn(editable);
    fireEvent.focus(editable);
    const codeText = document.createTreeWalker(container.querySelector("pre code")!, NodeFilter.SHOW_TEXT).nextNode()!;
    const range = document.createRange();
    range.setStart(codeText, 1);
    range.collapse(true);
    const selection = window.getSelection()!;
    selection.removeAllRanges();
    selection.addRange(range);
    document.dispatchEvent(new Event("selectionchange"));
  });

  const codeSettings = await waitFor(() => {
    const el = document.querySelector<HTMLElement>('[aria-label="代码块设置"]');
    expect(el).not.toBeNull();
    return el!;
  });
  const select = codeSettings.querySelector<HTMLInputElement>('input[role="combobox"]')!;
  expect(select).toHaveValue("custom <dialect>");
  fireEvent.keyDown(select, { key: "ArrowDown" });
  const option = await screen.findByRole("option", { name: "JavaScript" });
  fireEvent.pointerDown(option, { pointerType: "mouse" });
  fireEvent.click(option);
  await waitFor(() => expect(onChange).toHaveBeenCalled());
  expect(onChange.mock.lastCall![0].blocks[0]).toEqual({ ...initial.blocks[1], language: "javascript" });
});

it("t29_3: uploads replacement image via BlockEditor ImageForm with multipart/CSRF/Idempotency-Key and persists only asset_id", async () => {
  const requests: Request[] = [];
  const fetcher: typeof fetch = async (input) => {
    requests.push(input as Request);
    return response();
  };
  const { onChange } = await setup(initial, fetcher);
  document.cookie = "knoted_csrf=public-fixture-csrf; path=/";

  const formGroup = await openImageEditForm();
  const file = new File(["public PNG fixture"], "pixel.png", { type: "image/png" });
  fireEvent.change(screen.getByLabelText("上传图片"), { target: { files: [file] } });

  await waitFor(() =>
    expect(screen.getByLabelText("图片地址")).toHaveValue(
      `${window.location.origin}/api/v1/assets/${asset.id}/content`,
    ),
  );
  expect(onChange).not.toHaveBeenCalled();

  fireEvent.click(within(formGroup).getByText("应用图片"));
  await waitFor(() => expect(onChange).toHaveBeenCalledTimes(1));
  const saved = onChange.mock.lastCall![0] as NoteDocument;
  expect(saved.blocks[0]).toEqual({ ...initial.blocks[0], asset_id: asset.id });
  expect(JSON.stringify(saved)).not.toContain("/api/");

  expect(requests).toHaveLength(1);
  expect(requests[0]!.url).toBe("http://localhost:3000/api/v1/assets");
  expect(requests[0]!.method).toBe("POST");
  expect(requests[0]!.credentials).toBe("same-origin");
  expect(requests[0]!.headers.get("X-CSRF-Token")).toBe("public-fixture-csrf");
  expect(requests[0]!.headers.get("Idempotency-Key")).toMatch(/^[0-9a-f-]{36}$/);
  expect(requests[0]!.headers.get("Content-Type")).toMatch(/^multipart\/form-data; boundary=/);
  const body = await requests[0]!.formData();
  expect(body.get("file")).toMatchObject({ name: "pixel.png", type: "image/png" });
});

it("failed upload or reclaimed asset keeps old image, shows feedback, and never calls onChange", async () => {
  const fetcher: typeof fetch = async () =>
    new Response(JSON.stringify({ ...asset, state: "deleted" }), {
      status: 201,
      headers: { "Content-Type": "application/json" },
    });
  const { onChange } = await setup(initial, fetcher);
  await openImageEditForm();
  const file = new File(["public PNG fixture"], "retired.png", { type: "image/png" });
  fireEvent.change(screen.getByLabelText("上传图片"), { target: { files: [file] } });

  expect(await screen.findByText("图片已回收，请重新选择文件")).toBeInTheDocument();
  expect(screen.getByLabelText("图片地址")).toHaveValue(
    `${window.location.origin}/api/v1/assets/${assetId}/content`,
  );
  expect(onChange).not.toHaveBeenCalled();
});

it("refuses saving an unuploaded external image URL entered into ImageForm", async () => {
  const { onChange } = await setup(initial);
  const formGroup = await openImageEditForm();
  fireEvent.change(screen.getByLabelText("图片地址"), {
    target: { value: "https://example.com/external.png" },
  });
  fireEvent.click(within(formGroup).getByText("应用图片"));

  expect(await screen.findByText(/未应用此次编辑.*上传图片/)).toBeInTheDocument();
  expect(onChange).not.toHaveBeenCalled();
});

it("aborts in-flight image upload when NoteEditor unmounts or switches documentKey", async () => {
  const pending = deferred();
  let signal!: AbortSignal;
  const { onChange, unmount } = await setup(initial, async (input) => {
    signal = (input as Request).signal;
    return pending.promise;
  });
  await openImageEditForm();
  const file = new File(["public PNG fixture"], "late.png", { type: "image/png" });
  fireEvent.change(screen.getByLabelText("上传图片"), { target: { files: [file] } });
  await waitFor(() => expect(signal).toBeDefined());

  unmount();
  await act(async () => { pending.resolve(response()); });
  expect(signal.aborted).toBe(true);
  expect(onChange).not.toHaveBeenCalled();
});

it("uploadNoteImage rejects non-ready responses and invalid asset IDs", async () => {
  await stubMultipartGlobals();
  const badClient = createApiClient({
    baseUrl: "/api/v1",
    fetch: async () => new Response(JSON.stringify({ ...asset, id: "not-a-uuid" }), {
      status: 201,
      headers: { "Content-Type": "application/json" },
    }),
  });
  await expect(uploadNoteImage(badClient, new File(["x"], "x.png", { type: "image/png" }))).rejects.toThrow("图片资产响应无效");
});
