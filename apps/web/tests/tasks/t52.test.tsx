import { StrictMode } from "react";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { createApiClient, type components } from "@knoted/api-client";
import { build, loadConfigFromFile } from "vite";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { FilePreview, type FilePreviewProps } from "../../src/features/files/preview/FilePreview";
import type { FileSource } from "../../src/features/files/preview/inspect";
import { startOfficePreviewHost } from "../../src/features/files/preview/office-preview-host";
import { UiProvider } from "../../src/lib/ui-provider";

const origin = "https://docs.example.test:8443";
const scriptUrl = `${origin}/web-apps/apps/api/documents/api.js`;
const first = "00000000-0000-4000-8000-000000000052";
const second = "00000000-0000-4000-8000-000000000053";
const a: FileSource = {
    id: "a", name: "book.XLSX", relative_path: "book.XLSX", size: 42, mime: null, sha256: "a".repeat(64),
    version: 7, content_version: 3, status: "present", note: null, tag_ids: [],
    created_at: "2026-11-18T00:00:00Z", updated_at: "2026-11-18T00:00:00Z", last_seen_at: null,
};
const b: FileSource = { ...a, id: "b", name: "other.xlsx", relative_path: "other.xlsx", content_version: 4 };
type Preview = components["schemas"]["OfficePreview"];
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
function deferred<T>() {
    let resolve!: (value: T) => void;
    const promise = new Promise<T>(r => { resolve = r; });
    return { promise, resolve };
}

// JSDOM has no browser credentialless support. Emulate only the detection and reflected setter.
function supportCredentialless() {
    Object.defineProperty(HTMLIFrameElement.prototype, "credentialless", {
        configurable: true,
        get(this: HTMLIFrameElement) { return this.hasAttribute("credentialless"); },
        set(this: HTMLIFrameElement, value: boolean) { this.toggleAttribute("credentialless", value); },
    });
}
const ports = new Set<MessagePort>();
beforeEach(() => {
    vi.stubEnv("KNOTED_OFFICE_ORIGIN", origin);
    vi.spyOn(window, "scrollTo").mockImplementation(() => {});
});
afterEach(() => {
    cleanup();
    for (const port of ports) port.close();
    ports.clear();
    Reflect.deleteProperty(HTMLIFrameElement.prototype, "credentialless");
    vi.useRealTimers();
    vi.unstubAllEnvs();
    vi.restoreAllMocks();
});

type Call = { method: string; path: string; query: string; body: unknown };
function harness(respond: (call: Call) => Response | Promise<Response> = () => created(), strict = false) {
    const calls: Call[] = [];
    const onUnauthorized = vi.fn();
    const props: FilePreviewProps = {
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
    const view = (changes: Partial<FilePreviewProps> = {}) => {
        const tree = <UiProvider><FilePreview {...props} {...changes}/></UiProvider>;
        return strict ? <StrictMode>{tree}</StrictMode> : tree;
    };
    const result = render(view());
    return { ...result, calls, onUnauthorized, props, change: (changes: Partial<FilePreviewProps>) => result.rerender(view(changes)) };
}
const revocations = (calls: Call[]) => calls.filter(call => call.method === "DELETE").map(call => `${call.path}${call.query}`);
const lease = (file: string, id: string) => `/api/v1/files/${file}/office-preview?preview_id=${id}`;
const frame = () => document.querySelector<HTMLIFrameElement>('iframe[src="/office-preview.html"]');
const close = () => fireEvent.click(screen.getByRole("button", { name: "关闭预览" }));
const retry = () => fireEvent.click(screen.getByRole("button", { name: "重试 Office 预览" }));
const loading = () => screen.queryByLabelText("正在读取 Office 预览");
const sdkIn = (doc: Document) => doc.querySelector<HTMLScriptElement>(`script[src="${scriptUrl}"]`);
function settle() { return new Promise<void>(resolve => setImmediate(resolve)); }
async function attachHost(acknowledge = true) {
    await act(settle);
    expect(frame()).not.toBeNull();
    const host = frame()!;
    const target = host.contentWindow!;
    const incoming: unknown[] = [];
    const post = vi.spyOn(target, "postMessage").mockImplementation(() => {});
    fireEvent.load(host);
    expect(post).toHaveBeenCalledExactlyOnceWith({ type: "knoted-office-connect" }, window.location.origin, [expect.any(MessagePort)]);
    const [[, , [port]]] = post.mock.calls as unknown as [[unknown, string, [MessagePort]]];
    ports.add(port);
    port.onmessage = event => incoming.push(event.data);
    if (acknowledge) port.postMessage({ type: "connected" });
    return { host, port, incoming };
}

it("isolates before insertion and waits for private handshake before POST; no config appears in host URL", async () => {
    supportCredentialless();
    const append = Element.prototype.append;
    const atInsertion = vi.fn();
    vi.spyOn(Element.prototype, "append").mockImplementation(function (this: Element, ...nodes) {
        for (const node of nodes) if (node instanceof HTMLIFrameElement) atInsertion(node.hasAttribute("credentialless"), node.getAttribute("src"));
        append.apply(this, nodes);
    });
    const h = harness(undefined, true);
    const host = await attachHost(false);
    expect(atInsertion).toHaveBeenCalledExactlyOnceWith(true, "/office-preview.html");
    expect(h.calls).toEqual([]);
    // A window-channel spoof and an invalid private envelope cannot start the authenticated request.
    await act(async () => {
        window.dispatchEvent(new MessageEvent("message", { origin: location.origin, source: host.host.contentWindow, data: { type: "connected" } }));
        host.port.postMessage({ type: "connected", cookie: "synthetic" });
        await settle();
    });
    expect(h.calls).toEqual([]);
    host.port.postMessage({ type: "connected" });
    host.port.postMessage({ type: "connected" });
    await waitFor(() => expect(host.incoming).toEqual([{ type: "init", origin, config: signed().config }]));
    expect(h.calls).toEqual([{ method: "POST", path: "/api/v1/files/a/office-preview", query: "", body: { expected_content_version: 3 } }]);
    expect(sdkIn(document)).toBeNull();
    expect(loading()).not.toBeNull();
    await act(async () => { host.port.postMessage({ type: "ready" }); await settle(); });
    await waitFor(() => expect(loading()).toBeNull());
    fireEvent.click(screen.getByRole("button", { name: "下载原文件" }));
    expect(h.props.onDownload).toHaveBeenCalledWith(a);
});

it("fails closed without credentialless support but keeps ordinary download", async () => {
    const h = harness();
    await screen.findByText(/浏览器不支持.*Office/);
    expect(frame()).toBeNull();
    expect(h.calls).toHaveLength(0);
    expect(sdkIn(document)).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "下载原文件" }));
    expect(h.props.onDownload).toHaveBeenCalledWith(a);
});

it("disabled Office requests neither host nor lease", async () => {
    supportCredentialless();
    vi.stubEnv("KNOTED_OFFICE_ORIGIN", "");
    const h = harness();
    await screen.findByText(/Office 预览未启用/);
    expect(frame()).toBeNull();
    expect(h.calls).toHaveLength(0);
});

it.each(["", "http://docs.example.test:8443", "https://other.example.test:8443", "https://docs.example.test", "https://docs.example.test:8444", `${origin}/`])("never transfers a lease with a mismatched origin %j", async server => {
    supportCredentialless();
    const h = harness(() => created(first, server));
    const host = await attachHost();
    await screen.findByText(/Office 预览来源不匹配/);
    expect(host.incoming).toEqual([]);
    expect(frame()).toBeNull();
    await waitFor(() => expect(revocations(h.calls)).toEqual([lease("a", first)]));
});

it("a port error removes the host, revokes its lease and a retry cannot consume stale status", async () => {
    supportCredentialless();
    let requests = 0;
    const h = harness(() => created(++requests === 1 ? first : second));
    const old = await attachHost();
    await waitFor(() => expect(old.incoming).toHaveLength(1));
    await act(async () => {
        old.port.postMessage({ type: "error", diagnostic: "should-not-be-rendered" });
        await settle();
    });
    expect(frame()).toBe(old.host);
    old.port.postMessage({ type: "error" });
    await screen.findByRole("button", { name: "重试 Office 预览" });
    expect(frame()).toBeNull();
    await waitFor(() => expect(revocations(h.calls)).toEqual([lease("a", first)]));
    retry();
    const fresh = await attachHost();
    expect(fresh.host).not.toBe(old.host);
    await waitFor(() => expect(fresh.incoming).toEqual([{ type: "init", origin, config: signed(second).config }]));
    await act(async () => { old.port.postMessage({ type: "ready" }); old.port.postMessage({ type: "error" }); await settle(); });
    expect(loading()).not.toBeNull();
    expect(frame()).toBe(fresh.host);
    close();
    expect(frame()).toBeNull();
    await waitFor(() => expect(revocations(h.calls)).toEqual([lease("a", first), lease("a", second)]));
});

it("times out a silent host at 45s without a lease and enables retry", async () => {
    supportCredentialless();
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const h = harness();
    await attachHost(false);
    await act(async () => vi.advanceTimersByTimeAsync(44_999));
    expect(loading()).not.toBeNull();
    await act(async () => vi.advanceTimersByTimeAsync(1));
    expect(screen.getByText(/Office 预览启动超时/)).toBeInTheDocument();
    expect(frame()).toBeNull();
    expect(h.calls).toEqual([]);
});

it("late POST success after timeout only revokes its own ID; ready keeps the next host beyond 45s", async () => {
    supportCredentialless();
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const late = deferred<Response>();
    const h = harness(call => call.path.includes("/a/") ? late.promise : created(second));
    const old = await attachHost();
    await vi.waitFor(() => expect(h.calls).toHaveLength(1));
    await act(async () => vi.advanceTimersByTimeAsync(45_000));
    expect(frame()).toBeNull();
    h.change({ file: b });
    const fresh = await attachHost();
    await vi.waitFor(() => expect(fresh.incoming).toHaveLength(1));
    await act(async () => late.resolve(created()));
    await vi.waitFor(() => expect(revocations(h.calls)).toEqual([lease("a", first)]));
    expect(old.incoming).toEqual([]);
    expect(frame()).toBe(fresh.host);
    await act(async () => { fresh.port.postMessage({ type: "ready" }); await settle(); });
    await act(async () => vi.advanceTimersByTimeAsync(45_000));
    expect(loading()).toBeNull();
    expect(frame()).toBe(fresh.host);
});

it("times out and revokes an already issued lease if the viewer never reports ready", async () => {
    supportCredentialless();
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const h = harness();
    const host = await attachHost();
    await vi.waitFor(() => expect(host.incoming).toHaveLength(1));
    await act(async () => vi.advanceTimersByTimeAsync(45_000));
    expect(frame()).toBeNull();
    expect(screen.getByText(/Office 预览启动超时/)).toBeInTheDocument();
    await vi.waitFor(() => expect(revocations(h.calls)).toEqual([lease("a", first)]));
});

const unauthorized = () => Response.json({ code: "unauthorized", message: "session ended", request_id: first }, { status: 401 });
it.each(["POST", "GET"])("%s 401 closes the host via the existing client; revokes any obtained lease", async method => {
    supportCredentialless();
    const h = harness(call => call.method === method ? unauthorized() : created());
    h.change({ onDownload: async file => {
        await h.props.api.raw.GET("/files/{fileId}/content", { params: { path: { fileId: file.id } } });
    } });
    const host = await attachHost();
    if (method === "GET") {
        await waitFor(() => expect(host.incoming).toHaveLength(1));
        fireEvent.click(screen.getByRole("button", { name: "下载原文件" }));
    }
    await waitFor(() => expect(h.onUnauthorized).toHaveBeenCalledOnce());
    await waitFor(() => expect(h.props.onOpenChange).toHaveBeenCalledWith(false));
    expect(frame()).toBeNull();
    expect(screen.queryByRole("dialog")).toBeNull();
    await waitFor(() => expect(revocations(h.calls)).toEqual(method === "GET" ? [lease("a", first)] : []));
});

describe("Vite platform origin and emitted HTML", () => {
    it.each(["https://[::1]:8443", "http://docs.example.test", "https://*.example.test", "https://test;script-src", "https://user@docs.example.test", `${origin}/path`, `${origin}?q`, `${origin}#f`, "https://docs.example.test:443"])("rejects invalid build setting %j", async value => {
        vi.stubEnv("KNOTED_OFFICE_ORIGIN", value);
        await expect(loadConfigFromFile({ command: "build", mode: "production" })).rejects.toThrow(/KNOTED_OFFICE_ORIGIN/);
    });

    it.each([origin, ""])("emits both real HTML policies before script for origin %j", async configured => {
        vi.stubEnv("KNOTED_OFFICE_ORIGIN", configured);
        const result = await build({ logLevel: "silent", build: { write: false, minify: false } });
        const entries = [result].flat().flatMap(output => "output" in output ? output.output : []);
        const getHtml = (filename: string) => {
            const file = entries.find(item => item.type === "asset" && item.fileName === filename);
            expect(file, `static entry ${filename}`).toBeDefined();
            if (file?.type !== "asset") throw new Error(`Expected HTML asset ${filename}`);
            const html = new DOMParser().parseFromString(String(file.source), "text/html");
            const tags = html.querySelectorAll('meta[http-equiv="Content-Security-Policy"]');
            expect(tags).toHaveLength(1);
            const meta = tags[0]!;
            expect(html.head.querySelector("script")?.type).toBe("module");
            expect(meta.compareDocumentPosition(html.head.querySelector("script")!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
            return meta.getAttribute("content");
        };
        expect(getHtml("index.html")).toBe("script-src 'self'; frame-src 'self' blob:; object-src 'none'");
        expect(getHtml("office-preview.html")).toBe(configured
            ? `script-src 'self' ${origin}; frame-src ${origin}; object-src 'none'`
            : "script-src 'self'; frame-src 'none'; object-src 'none'");
    }, 20_000);
});

it("host requires isolating embedding and an exact parent/port before loading and forwarding SDK events", async () => {
    const element = document.createElement("iframe");
    element.src = "/office-preview.html";
    document.body.append(element);
    const host = element.contentWindow!;
    const doc = host.document;
    doc.open();
    doc.write('<!doctype html><head></head><body><main id="office-preview-host"></main></body>');
    doc.close();
    const channel = () => {
        const c = new MessageChannel();
        ports.add(c.port1); ports.add(c.port2);
        return c;
    };
    const connect = (port: MessagePort, from: Window = host.parent, senderOrigin = location.origin) => {
        host.dispatchEvent(new MessageEvent("message", { data: { type: "knoted-office-connect" }, source: from, origin: senderOrigin, ports: [port] }));
    };
    try {
        startOfficePreviewHost(window); // Direct navigation must not load anything.
        expect(sdkIn(document)).toBeNull();
        startOfficePreviewHost(host); // Ordinary iframe: must not even bind.
        const ordinary = channel();
        const rejected = vi.fn();
        ordinary.port1.onmessage = rejected;
        connect(ordinary.port2);
        await settle();
        expect(rejected).not.toHaveBeenCalled();
        expect(sdkIn(doc)).toBeNull();

        Object.defineProperty(host, "credentialless", { value: true });
        expect(host.location.origin).toEqual(window.location.origin);
        expect(doc.getElementById("office-preview-host")).not.toBeNull();
        startOfficePreviewHost(host);
        connect(ordinary.port2, host);
        connect(ordinary.port2, host.parent, "https://elsewhere.example.test");
        const valid = channel();
        const status: unknown[] = [];
        valid.port1.onmessage = event => status.push(event.data);
        connect(valid.port2);
        connect(ordinary.port2); // Reject all subsequent ports, even from same parent.
        await vi.waitFor(() => expect(status).toEqual([{ type: "connected" }]));
        ordinary.port1.postMessage({ type: "init", origin, config: signed().config });
        valid.port1.postMessage({ type: "init", origin, config: signed().config, cookie: "synthetic" });
        await settle();
        expect(sdkIn(doc)).toBeNull();
        expect(rejected).not.toHaveBeenCalled();
        valid.port1.postMessage({ type: "init", origin, config: signed().config });
        valid.port1.postMessage({ type: "init", origin, config: signed(second).config });
        await vi.waitFor(() => expect(sdkIn(doc)).not.toBeNull());
        const destroyEditor = vi.fn();
        const init = vi.fn();
        Object.defineProperty(host, "DocsAPI", { value: { DocEditor: class {
            constructor(id: string, config: unknown) { init(id, config); }
            destroyEditor = destroyEditor;
        } } });
        fireEvent.load(sdkIn(doc)!);
        expect(init).toHaveBeenCalledExactlyOnceWith(expect.any(String), { ...signed().config, width: "100%", height: "100%", events: { onDocumentReady: expect.any(Function), onError: expect.any(Function) } });
        const events = init.mock.calls[0]![1].events;
        events.onDocumentReady();
        await vi.waitFor(() => expect(status).toEqual([{ type: "connected" }, { type: "ready" }]));
        events.onError({ data: "private-sdk-diagnostic" });
        await vi.waitFor(() => expect(status).toEqual([{ type: "connected" }, { type: "ready" }, { type: "error" }]));
        expect(destroyEditor).toHaveBeenCalledOnce();
    } finally {
        host.dispatchEvent(new Event("pagehide"));
        element.remove();
    }
});
