import { StrictMode } from "react";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { components } from "@knoted/api-client";
import { loadConfigFromFile } from "vite";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { FilePreviewProps } from "../../src/features/files/preview/FilePreview";
import type { FileSource } from "../../src/features/files/preview/inspect";
import { UiProvider } from "../../src/lib/ui-provider";

const origin = "https://docs.example.test:8443";
const scriptUrl = `${origin}/web-apps/apps/api/documents/api.js`;
const first = "00000000-0000-4000-8000-000000000052";
const second = "00000000-0000-4000-8000-000000000053";
const third = "00000000-0000-4000-8000-000000000054";
const a: FileSource = {
    id: "a", name: "book.XLSX", relative_path: "book.XLSX", size: 42, mime: null, sha256: "a".repeat(64),
    version: 7, content_version: 3, status: "present", note: null, tag_ids: [],
    created_at: "2026-11-18T00:00:00Z", updated_at: "2026-11-18T00:00:00Z", last_seen_at: null,
};
const b: FileSource = { ...a, id: "b", name: "other.xlsx", relative_path: "other.xlsx", content_version: 4 };
type Preview = components["schemas"]["OfficePreview"];
type SdkConfig = Preview["config"] & {
    events: { onDocumentReady(): void; onError(error: unknown): void };
};
function signed(id = first, server = origin): Preview {
    return {
        preview_id: id, expires_at: "2026-11-18T00:02:00Z", document_server_origin: server,
        config: {
            documentType: "cell",
            document: {
                fileType: "xlsx", key: id, title: "book.XLSX", url: `https://knoted.example.test/api/v1/office-preview-sources/${id}`,
                permissions: {
                    edit: false, review: false, comment: false, fillForms: false, modifyFilter: false, modifyContentControl: false,
                    chat: false, download: false, print: false, copy: false, changeHistory: false, protect: false,
                },
            },
            editorConfig: { mode: "view", customization: { plugins: false, macros: false, macrosMode: "disable" } },
            token: "fixture-only-not-a-jwt",
        },
    };
}
const created = (id?: string, server?: string) => Response.json(signed(id, server), { status: 201 });
const scripts = () => [...document.head.querySelectorAll<HTMLScriptElement>("script[src]")];
function deferred<T>() {
    let resolve!: (value: T) => void;
    const promise = new Promise<T>(r => { resolve = r; });
    return { promise, resolve };
}
function sdk(init?: (config: SdkConfig) => void) {
    const instances: { config: SdkConfig; destroy: ReturnType<typeof vi.fn> }[] = [];
    class DocEditor {
        destroyEditor: ReturnType<typeof vi.fn>;
        constructor(id: string, config: SdkConfig) {
            // The real SDK replaces its placeholder; React must not own that node.
            const placeholder = document.getElementById(id);
            if (!placeholder) throw new Error("Missing editor host");
            const frame = document.createElement("iframe");
            frame.title = "Fake Office viewer";
            placeholder.replaceWith(frame);
            this.destroyEditor = vi.fn(() => frame.remove());
            init?.(config);
            instances.push({ config, destroy: this.destroyEditor });
        }
    }
    vi.stubGlobal("DocsAPI", { DocEditor });
    return instances;
}

let FilePreview: typeof import("../../src/features/files/preview/FilePreview").FilePreview;
let createApiClient: typeof import("@knoted/api-client").createApiClient;
beforeEach(async () => {
    vi.resetModules();
    vi.stubEnv("KNOTED_OFFICE_ORIGIN", origin);
    vi.spyOn(window, "scrollTo").mockImplementation(() => {});
    // The client and dynamically imported preview must share the same error class.
    ({ createApiClient } = await import("@knoted/api-client"));
    ({ FilePreview } = await import("../../src/features/files/preview/FilePreview"));
});
afterEach(() => {
    cleanup();
    scripts().forEach(script => script.remove());
    vi.unstubAllGlobals();
    vi.unstubAllEnvs();
    vi.restoreAllMocks();
});

type Call = { method: string; path: string; query: string; body: unknown };
function harness(respond: (call: Call) => Response | Promise<Response> = () => created(), strict = false) {
    const calls: Call[] = [];
    const onUnauthorized = vi.fn();
    const defaults: FilePreviewProps = {
        file: a, open: true, onOpenChange: vi.fn(), onDownload: vi.fn(),
        api: createApiClient({
            baseUrl: "http://localhost/api/v1", onUnauthorized,
            async fetch(input) {
                const request = input as Request;
                const url = new URL(request.url);
                const call = { method: request.method, path: url.pathname, query: url.search, body: request.method === "POST" ? await request.json() : null };
                calls.push(call);
                return call.method === "DELETE" ? new Response(null, { status: 204 }) : respond(call);
            },
        }),
    };
    const view = (props: Partial<FilePreviewProps> = {}) => {
        const tree = <UiProvider><FilePreview {...defaults} {...props}/></UiProvider>;
        return strict ? <StrictMode>{tree}</StrictMode> : tree;
    };
    const result = render(view());
    return { ...result, calls, onUnauthorized, props: defaults, change: (props: Partial<FilePreviewProps>) => result.rerender(view(props)) };
}
const revocations = (calls: Call[]) => calls.filter(call => call.method === "DELETE").map(call => `${call.path}${call.query}`);
const lease = (file: string, token: string) => `/api/v1/files/${file}/office-preview?preview_id=${token}`;
async function loadScript(event = "load") {
    await waitFor(() => expect(scripts().map(script => script.src)).toEqual([scriptUrl]));
    await act(async () => { fireEvent(scripts()[0]!, new Event(event)); });
}
const close = () => fireEvent.click(screen.getByRole("button", { name: "关闭预览" }));
const retry = () => fireEvent.click(screen.getByRole("button", { name: "重试 Office 预览" }));

describe("T52 Office preview", () => {
    it("t52_1 uses the signed cell config unchanged and lets the SDK report document readiness", async () => {
        const instances = sdk();
        const h = harness();
        await loadScript();
        expect(h.calls).toEqual([{ method: "POST", path: "/api/v1/files/a/office-preview", query: "", body: { expected_content_version: 3 } }]);
        expect(instances).toHaveLength(1);
        const { events, ...fields } = instances[0]!.config;
        expect(fields).toEqual({ ...signed().config, height: "100%", width: "100%" });
        expect(screen.getByLabelText("正在读取 Office 预览")).toBeInTheDocument();
        act(() => events.onDocumentReady());
        expect(screen.queryByLabelText("正在读取 Office 预览")).toBeNull();
        expect(screen.getByTitle("Fake Office viewer")).toBeInTheDocument();
        fireEvent.click(screen.getByRole("button", { name: "下载原文件" }));
        expect(h.props.onDownload).toHaveBeenCalledWith(a);
    });

    it("t52_2 switches ownership before a late 201, then revokes each exact ID without cross-revocation", async () => {
        const late = deferred<Response>();
        const instances = sdk();
        const h = harness(call => call.path.includes("/a/") ? late.promise : created(second));
        await waitFor(() => expect(h.calls).toHaveLength(1));
        h.change({ file: b });
        await loadScript();
        expect(instances).toHaveLength(1);
        await act(async () => late.resolve(created()));
        await waitFor(() => expect(revocations(h.calls)).toEqual([lease("a", first)]));
        expect(instances).toHaveLength(1);
        expect(instances[0]!.config.document.key).toBe(second);
        close();
        expect(instances[0]!.destroy).toHaveBeenCalledOnce();
        await waitFor(() => expect(revocations(h.calls)).toEqual([lease("a", first), lease("b", second)]));
        act(() => instances[0]!.config.events.onError({ data: "late editor error" }));
        expect(screen.queryByRole("dialog")).toBeNull();
    });

    it("t52_2 revokes a POST that succeeds only after closing, without inserting a script or SDK host", async () => {
        const late = deferred<Response>();
        const instances = sdk();
        const h = harness(() => late.promise);
        await waitFor(() => expect(h.calls).toHaveLength(1));
        close();
        await act(async () => late.resolve(created()));
        await waitFor(() => expect(revocations(h.calls)).toEqual([lease("a", first)]));
        expect(instances).toHaveLength(0);
        expect(scripts()).toHaveLength(0);
    });

    it.each(["", "http://docs.example.test:8443", "https://other.example.test:8443", "https://docs.example.test", "https://docs.example.test:8444", `${origin}/`, "https://docs.example.test:8443@other.example.test", `${origin};`])("t52_3 rejects a nonidentical response origin %j without loading anything", async server => {
        const instances = sdk();
        const h = harness(() => created(first, server));
        await screen.findByText(/Office 预览来源不匹配/);
        await waitFor(() => expect(revocations(h.calls)).toEqual([lease("a", first)]));
        expect(scripts()).toHaveLength(0);
        expect(instances).toHaveLength(0);
    });

    it("t52_3 disabled Office does not request a lease or script and keeps download available", async () => {
        vi.stubEnv("KNOTED_OFFICE_ORIGIN", "");
        const h = harness();
        await screen.findByText(/Office 预览未启用/);
        expect(h.calls).toHaveLength(0);
        expect(scripts()).toHaveLength(0);
        fireEvent.click(screen.getByRole("button", { name: "下载原文件" }));
        expect(h.props.onDownload).toHaveBeenCalledWith(a);
    });

    it("t52_4 retries a failed script with a new lease, then reuses the loaded script through StrictMode reopen", async () => {
        let requests = 0;
        const instances = sdk();
        const h = harness(() => created([first, second, third][requests++]!), true);
        await loadScript("error");
        await screen.findByRole("button", { name: "重试 Office 预览" });
        expect(scripts()).toHaveLength(0);
        await waitFor(() => expect(revocations(h.calls)).toEqual([lease("a", first)]));
        retry();
        await loadScript();
        expect(instances).toHaveLength(1);
        h.change({ file: b });
        expect(instances[0]!.destroy).toHaveBeenCalledOnce();
        await waitFor(() => expect(instances).toHaveLength(2));
        expect(scripts().map(s => s.src)).toEqual([scriptUrl]);
        h.unmount();
        expect(instances[1]!.destroy).toHaveBeenCalledOnce();
        await waitFor(() => expect(revocations(h.calls)).toEqual([lease("a", first), lease("a", second), lease("b", third)]));
        expect(h.calls.filter(call => call.method === "POST")).toHaveLength(3);
    });

    it("t52_4 retries a partial constructor failure or SDK error without displaying SDK-supplied error text", async () => {
        let init = 0;
        let requests = 0;
        const instances = sdk(() => { if (++init === 1) throw new Error("private-sdk-diagnostic"); });
        const h = harness(() => created(++requests === 1 ? first : second));
        await loadScript();
        await screen.findByRole("button", { name: "重试 Office 预览" });
        expect(screen.queryByTitle("Fake Office viewer")).toBeNull();
        expect(screen.queryByText(/private-sdk-diagnostic/)).toBeNull();
        await waitFor(() => expect(revocations(h.calls)).toEqual([lease("a", first)]));
        retry();
        await screen.findByTitle("Fake Office viewer");
        act(() => instances[0]!.config.events.onError({ data: "private-sdk-diagnostic" }));
        expect(instances[0]!.destroy).toHaveBeenCalledOnce();
        await screen.findByRole("button", { name: "重试 Office 预览" });
        expect(screen.queryByText(/private-sdk-diagnostic/)).toBeNull();
        await waitFor(() => expect(revocations(h.calls)).toEqual([lease("a", first), lease("a", second)]));
        expect(scripts().map(s => s.src)).toEqual([scriptUrl]);
    });

    const unauthorized = () => Response.json({ code: "unauthorized", message: "session ended", request_id: first }, { status: 401 });
    it("t52_4 a creation 401 closes the dialog through the existing client handler", async () => {
        const h = harness(unauthorized);
        await waitFor(() => expect(h.onUnauthorized).toHaveBeenCalledOnce());
        await waitFor(() => expect(h.props.onOpenChange).toHaveBeenCalledWith(false));
        expect(screen.queryByRole("dialog")).toBeNull();
        expect(revocations(h.calls)).toHaveLength(0);
        expect(scripts()).toHaveLength(0);
    });

    it("t52_4 a download 401 closes and releases an already initialized editor", async () => {
        const instances = sdk();
        const h = harness(call => call.method === "GET" ? unauthorized() : created());
        h.change({ onDownload: async file => {
            await h.props.api.raw.GET("/files/{fileId}/content", { params: { path: { fileId: file.id } } });
        } });
        await loadScript();
        fireEvent.click(screen.getByRole("button", { name: "下载原文件" }));
        await waitFor(() => expect(h.onUnauthorized).toHaveBeenCalledOnce());
        await waitFor(() => expect(revocations(h.calls)).toEqual([lease("a", first)]));
        expect(instances[0]!.destroy).toHaveBeenCalledOnce();
        expect(screen.queryByRole("dialog")).toBeNull();
    });
});

describe("t52_3 Vite platform origin", () => {
    const resolve = () => loadConfigFromFile({ command: "build", mode: "production" });
    it("publishes only the exact configured origin", async () => {
        expect((await resolve())?.config.define?.["import.meta.env.KNOTED_OFFICE_ORIGIN"]).toBe(JSON.stringify(origin));
    });
    it.each(["http://docs.example.test", "https://*.example.test", "https://test;script-src", "https://user@docs.example.test", `${origin}/path`, `${origin}?q`, `${origin}#f`, "https://docs.example.test:443"])("rejects invalid build setting %j", async value => {
        vi.stubEnv("KNOTED_OFFICE_ORIGIN", value);
        await expect(resolve()).rejects.toThrow(/KNOTED_OFFICE_ORIGIN/);
    });
});
