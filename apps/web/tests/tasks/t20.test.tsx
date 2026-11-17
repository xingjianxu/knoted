import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { createMemoryHistory } from "@tanstack/react-router";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import App from "../../src/App";
import { createAppRouter, safeReturnTo } from "../../src/lib/router";
import { createQueryClient, queryKeys } from "../../src/lib/query";
import { runWebAuthn } from "../../src/features/auth/webauthn";
import { UiProvider } from "../../src/lib/ui-provider";

class BrowserRequest extends Request {
  constructor(input: RequestInfo | URL, init?: RequestInit) {
    super(
      typeof input === "string"
        ? new URL(input, window.location.origin)
        : input,
      init,
    );
  }
}
const session = {
  session: {
    id: "s1",
    user_id: "u1",
    current: true,
    created_at: "2026-11-16T00:00:00Z",
    last_seen_at: "2026-11-16T00:00:00Z",
    idle_expires_at: "2026-11-23T00:00:00Z",
    absolute_expires_at: "2026-12-16T00:00:00Z",
  },
  csrf_token: "test-csrf-value-long",
};
let loggedIn = false;
let logoutFails = false;
const requests: string[] = [];
function json(data: unknown, status = 200) {
  return new Response(JSON.stringify(data), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}
beforeEach(() => {
  loggedIn = false;
  logoutFails = false;
  requests.length = 0;
  vi.stubGlobal("Request", BrowserRequest);
  vi.spyOn(window, "scrollTo").mockImplementation(() => {});
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL) => {
      const request = input as Request;
      const path = new URL(request.url).pathname;
      requests.push(`${request.method} ${path}`);
      if (path.endsWith("/auth/session"))
        return loggedIn
          ? json(session)
          : json({ code: "unauthorized", message: "未登录" }, 401);
      if (path.endsWith("/auth/login/options"))
        return json({
          ceremony_id: "c1",
          options: {
            publicKey: {
              challenge: "AQID",
              rpId: "localhost",
              allowCredentials: [{ id: "BAUG", type: "public-key" }],
            },
          },
        });
      if (path.endsWith("/auth/login/finish")) {
        loggedIn = true;
        return json(session);
      }
      if (path.endsWith("/auth/logout")) {
        if (logoutFails)
          return json({ code: "unavailable", message: "unavailable" }, 503);
        loggedIn = false;
        return new Response(null, { status: 204 });
      }
      if (path.endsWith("/auth/passkeys"))
        return json({ items: [], next_cursor: null });
      if (path.endsWith("/auth/sessions"))
        return json({ items: [session.session], next_cursor: null });
      throw new Error(`Unexpected request ${request.method} ${path}`);
    }),
  );
  Object.defineProperty(navigator, "credentials", {
    configurable: true,
    value: {
      get: vi.fn(async () => ({
        id: "credential",
        rawId: new Uint8Array([1, 2, 3]).buffer,
        type: "public-key",
        response: {
          clientDataJSON: new Uint8Array([4]).buffer,
          authenticatorData: new Uint8Array([5]).buffer,
          signature: new Uint8Array([6]).buffer,
          userHandle: null,
        },
        getClientExtensionResults: () => ({}),
      })),
    },
  });
});
afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});
async function mount(url: string, defaultBundle = false) {
  const initialBundle = defaultBundle ? undefined : createQueryClient();
  const router = createAppRouter({
    history: createMemoryHistory({ initialEntries: [url] }),
    bundle: initialBundle,
  });
  const bundle = router.options.context!;
  render(
    <UiProvider>
      <App router={router} />
    </UiProvider>,
  );
  await act(async () => {
    await router.load();
  });
  return { router, bundle };
}
describe("t20 auth routes", () => {
  it("t20_1 redirects protected routes with original destination", async () => {
    const { router } = await mount("/settings/security");
    expect(
      await screen.findByRole("button", { name: "使用 Passkey 登录" }),
    ).toBeInTheDocument();
    expect(router.state.location.pathname).toBe("/login");
    expect(router.state.location.search).toEqual({
      redirect: "/settings/security",
    });
    expect(
      screen.queryByRole("heading", { name: "凭据与会话" }),
    ).not.toBeInTheDocument();
  });
  it("t20_2 returns to original page after Passkey login", async () => {
    const { router } = await mount("/login?redirect=%2Fsettings%2Fsecurity");
    fireEvent.click(
      await screen.findByRole("button", { name: "使用 Passkey 登录" }),
    );
    expect(
      await screen.findByRole("heading", { name: "凭据与会话" }),
    ).toBeInTheDocument();
    expect(router.state.location.pathname).toBe("/settings/security");
  });
  it("t20_3 logout clears private queries and mutation cache", async () => {
    loggedIn = true;
    const { bundle } = await mount("/");
    bundle.queryClient.setQueryData(queryKeys.notes.detail("private"), {
      secret: "private",
    });
    bundle.queryClient
      .getMutationCache()
      .build(bundle.queryClient, { mutationKey: ["private"] });
    fireEvent.click(await screen.findByRole("button", { name: "退出登录" }));
    await screen.findByRole("button", { name: "使用 Passkey 登录" });
    expect(
      bundle.queryClient.getQueryData(queryKeys.notes.detail("private")),
    ).toBeUndefined();
    expect(bundle.queryClient.getMutationCache().getAll()).toHaveLength(0);
  });
  it("does not report failed logout as success", async () => {
    loggedIn = true;
    logoutFails = true;
    await mount("/");
    fireEvent.click(await screen.findByRole("button", { name: "退出登录" }));
    expect(await screen.findByRole("alert")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "退出登录" }),
    ).toBeInTheDocument();
    expect(loggedIn).toBe(true);
  });
  it("cancelled authenticator does not submit finish", async () => {
    vi.mocked(navigator.credentials.get).mockRejectedValue(
      new DOMException("cancelled", "NotAllowedError"),
    );
    await mount("/login");
    fireEvent.click(
      await screen.findByRole("button", { name: "使用 Passkey 登录" }),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent("取消");
    expect(requests).not.toContain("POST /api/v1/auth/login/finish");
  });
  it("rejects external, encoded and authentication return paths", () => {
    for (const path of [
      "https://evil.example/",
      "//evil.example/",
      "/\\evil.example/",
      "/login",
      "/recovery",
      "/%2F%2Fevil.example",
      "/unknown",
    ])
      expect(safeReturnTo(path)).toBe("/");
    expect(safeReturnTo("/settings/security")).toBe("/settings/security");
  });
  it("disables unimplemented navigation entries while notes is open", async () => {
    loggedIn = true;
    await mount("/");
    // T38 已把项目入口接入真实页面，其他入口仍待开放。
    for (const name of ["概览", "事项", "文件", "搜索", "问答"])
      expect(
        await screen.findByRole("button", { name: new RegExp(name) }),
      ).toBeDisabled();
    expect(screen.getByRole("button", { name: "笔记" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "项目" })).toBeEnabled();
  });
  it("network errors show retry without claiming unauthenticated", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        throw new TypeError("offline");
      }),
    );
    const { router, bundle } = await mount("/settings/security");
    await waitFor(() => expect(router.state.status).toBe("idle"), {
      timeout: 10000,
    });
    expect(await screen.findByRole("alert")).toHaveTextContent("重试");
    expect(router.state.location.pathname).toBe("/settings/security");
    bundle.queryClient.clear();
  });
});

describe("t20 credential management", () => {
  it("lists passkeys and prevents deleting the only credential", async () => {
    loggedIn = true;
    const baseFetch = globalThis.fetch;
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL) => {
        if (new URL((input as Request).url).pathname.endsWith("/auth/passkeys"))
          return json({
            items: [
              {
                id: "p1",
                name: "Device",
                created_at: "2026-11-16T00:00:00Z",
                last_used_at: null,
                credential_id: "a",
                transports: [],
              },
            ],
            next_cursor: null,
          });
        return baseFetch(input);
      }),
    );
    await mount("/settings/security");
    expect(await screen.findByText("Device")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "删除 Device" })).toBeDisabled();
  });
  it("uses opaque passkey cursors and restores previous page", async () => {
    loggedIn = true;
    const baseFetch = globalThis.fetch;
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL) => {
        const url = new URL((input as Request).url);
        if (url.pathname.endsWith("/auth/passkeys"))
          return json({
            items: [
              {
                id: url.searchParams.has("cursor") ? "p2" : "p1",
                name: url.searchParams.has("cursor") ? "Second" : "First",
                created_at: "2026-11-16T00:00:00Z",
                last_used_at: null,
                credential_id: "a",
                transports: [],
              },
            ],
            next_cursor: url.searchParams.has("cursor")
              ? null
              : "opaque+cursor",
          });
        return baseFetch(input);
      }),
    );
    await mount("/settings/security");
    expect(await screen.findByText("First")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "下一页凭据" }));
    expect(await screen.findByText("Second")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "上一页凭据" }));
    expect(await screen.findByText("First")).toBeInTheDocument();
  });
  it("registration decodes challenge/user/exclusions and encodes attestation", async () => {
    const create = vi.fn(async () => ({
      id: "new",
      rawId: new Uint8Array([1, 2]).buffer,
      type: "public-key",
      response: {
        clientDataJSON: new Uint8Array([3]).buffer,
        attestationObject: new Uint8Array([4]).buffer,
        getTransports: () => ["internal"],
      },
    }));
    Object.defineProperty(navigator, "credentials", {
      configurable: true,
      value: { create },
    });
    const result = await runWebAuthn(
      {
        publicKey: {
          challenge: "AQID",
          user: { id: "BAUG", name: "owner", displayName: "Owner" },
          rp: { name: "Knoted" },
          pubKeyCredParams: [{ alg: -7, type: "public-key" }],
          excludeCredentials: [{ id: "BwgJ", type: "public-key" }],
        },
      },
      "create",
    );
    const options = create.mock.calls[0] as unknown as [
      {
        publicKey: {
          challenge: ArrayBuffer;
          user: { id: ArrayBuffer };
          excludeCredentials: Array<{ id: ArrayBuffer }>;
        };
      },
    ];
    expect(Array.from(new Uint8Array(options[0].publicKey.challenge))).toEqual([
      1, 2, 3,
    ]);
    expect(Array.from(new Uint8Array(options[0].publicKey.user.id))).toEqual([
      4, 5, 6,
    ]);
    expect(result).toEqual({
      id: "new",
      rawId: "AQI",
      type: "public-key",
      response: {
        clientDataJSON: "Aw",
        attestationObject: "BA",
        transports: ["internal"],
      },
    });
  });
});

describe("t20 ceremony form contracts", () => {
  for (const mode of ["bootstrap", "recovery"] as const) {
    it(`${mode} uses a password token field and submits the public API contract`, async () => {
      const baseFetch = globalThis.fetch;
      const bodies: Array<{ path: string; body: Record<string, unknown> }> = [];
      vi.stubGlobal(
        "fetch",
        vi.fn(async (input: RequestInfo | URL) => {
          const request = input as Request;
          const path = new URL(request.url).pathname;
          if (path.endsWith(`/auth/${mode}/options`)) {
            bodies.push({
              path,
              body: (await request.json()) as Record<string, unknown>,
            });
            return json({
              ceremony_id: "c1",
              options: {
                publicKey: {
                  challenge: "AQID",
                  user: { id: "BAUG", name: "owner", displayName: "Owner" },
                  rp: { name: "Knoted" },
                  pubKeyCredParams: [{ alg: -7, type: "public-key" }],
                },
              },
            });
          }
          if (path.endsWith(`/auth/${mode}/finish`)) {
            bodies.push({
              path,
              body: (await request.json()) as Record<string, unknown>,
            });
            loggedIn = true;
            return json(session, 201);
          }
          return baseFetch(input);
        }),
      );
      Object.defineProperty(navigator, "credentials", {
        configurable: true,
        value: {
          create: vi.fn(async () => ({
            id: "new",
            rawId: new Uint8Array([1]).buffer,
            type: "public-key",
            response: {
              clientDataJSON: new Uint8Array([2]).buffer,
              attestationObject: new Uint8Array([3]).buffer,
            },
          })),
        },
      });
      const { router } = await mount(`/${mode}`);
      const token = await screen.findByLabelText("一次性令牌");
      expect(token).toHaveAttribute("type", "password");
      fireEvent.change(token, { target: { value: "one-use-test-token" } });
      fireEvent.click(
        screen.getByRole("button", {
          name: mode === "bootstrap" ? "注册 Passkey" : "替换凭据并恢复",
        }),
      );
      await screen.findByRole("button", { name: "退出登录" });
      expect(bodies[0]?.body).toEqual(
        mode === "bootstrap"
          ? { bootstrap_token: "one-use-test-token" }
          : { recovery_token: "one-use-test-token" },
      );
      expect(bodies[1]?.body).toMatchObject({
        ceremony_id: "c1",
        name: "我的 Passkey",
        credential: {
          id: "new",
          rawId: "AQ",
          type: "public-key",
          response: { clientDataJSON: "Ag", attestationObject: "Aw" },
        },
      });
      if (mode === "recovery")
        expect(bodies[1]?.body.recovery_token).toBe("one-use-test-token");
      else expect(bodies[1]?.body).not.toHaveProperty("bootstrap_token");
      expect(router.state.location.href).toBe("/");
      expect(sessionStorage.length).toBe(0);
      expect(localStorage.length).toBe(0);
    });
  }
  it("revoking current session clears caches and returns to login", async () => {
    loggedIn = true;
    const baseFetch = globalThis.fetch;
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL) => {
        const request = input as Request;
        if (
          request.method === "DELETE" &&
          new URL(request.url).pathname.endsWith("/auth/sessions/s1")
        ) {
          loggedIn = false;
          return new Response(null, { status: 204 });
        }
        return baseFetch(input);
      }),
    );
    const { bundle } = await mount("/settings/security");
    bundle.queryClient.setQueryData(
      queryKeys.notes.detail("secret"),
      "private",
    );
    fireEvent.click(
      await screen.findByRole("button", { name: "撤销当前会话" }),
    );
    fireEvent.click(await screen.findByRole("button", { name: "确认撤销" }));
    await screen.findByRole("button", { name: "使用 Passkey 登录" });
    expect(
      bundle.queryClient.getQueryData(queryKeys.notes.detail("secret")),
    ).toBeUndefined();
  });
});

describe("t20 runtime session revocation", () => {
  it("401 from protected API closes shell and drops private cache", async () => {
    loggedIn = true;
    const baseFetch = globalThis.fetch;
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL) => {
        if (!loggedIn)
          return json({ code: "unauthorized", message: "expired" }, 401);
        return baseFetch(input);
      }),
    );
    const { bundle } = await mount("/settings/security", true);
    await screen.findByRole("button", { name: "撤销当前会话" });
    bundle.queryClient.setQueryData(queryKeys.notes.detail("private"), {
      secret: "private",
    });
    loggedIn = false;
    await act(async () => {
      await bundle.queryClient.invalidateQueries({
        queryKey: queryKeys.auth.sessions(),
      });
    });
    expect(
      await screen.findByRole("button", { name: "使用 Passkey 登录" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("heading", { name: "凭据与会话" }),
    ).not.toBeInTheDocument();
    expect(
      bundle.queryClient.getQueryData(queryKeys.notes.detail("private")),
    ).toBeUndefined();
  });
});

describe("t20 ceremony lifecycle", () => {
  it("leaving login cancels an unfinished authenticator and never submits finish", async () => {
    let complete: ((credential: Credential | null) => void) | undefined;
    const get = vi.fn(
      () =>
        new Promise<Credential | null>((resolve) => {
          complete = resolve;
        }),
    );
    Object.defineProperty(navigator, "credentials", {
      configurable: true,
      value: { get },
    });
    const { router } = await mount("/login");
    fireEvent.click(
      await screen.findByRole("button", { name: "使用 Passkey 登录" }),
    );
    await waitFor(() => expect(get).toHaveBeenCalledOnce());
    await act(async () => {
      await router.navigate({ to: "/bootstrap" });
    });
    await screen.findByRole("button", { name: "注册 Passkey" });
    await act(async () => {
      complete?.({
        id: "late",
        rawId: new Uint8Array([1]).buffer,
        type: "public-key",
        response: {
          clientDataJSON: new Uint8Array([1]).buffer,
          authenticatorData: new Uint8Array([1]).buffer,
          signature: new Uint8Array([1]).buffer,
          userHandle: null,
        },
      } as unknown as Credential);
    });
    expect(requests).not.toContain("POST /api/v1/auth/login/finish");
    expect(router.state.location.pathname).toBe("/bootstrap");
  });
  it("unsupported authentication methods explain browser support", async () => {
    Object.defineProperty(navigator, "credentials", {
      configurable: true,
      value: {},
    });
    await mount("/login");
    fireEvent.click(
      await screen.findByRole("button", { name: "使用 Passkey 登录" }),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "不支持 Passkey",
    );
    expect(requests).not.toContain("POST /api/v1/auth/login/finish");
  });
});

describe("t20 protected mutation errors", () => {
  it("server rejection keeps the credential visible and explains the conflict", async () => {
    loggedIn = true;
    const baseFetch = globalThis.fetch;
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL) => {
        const request = input as Request;
        const path = new URL(request.url).pathname;
        if (path.endsWith("/auth/passkeys"))
          return json({
            items: ["Device A", "Device B"].map((name, i) => ({
              id: `p${i}`,
              name,
              created_at: "2026-11-16T00:00:00Z",
              last_used_at: null,
              credential_id: `c${i}`,
              transports: [],
            })),
            next_cursor: null,
          });
        if (request.method === "DELETE" && path.endsWith("/auth/passkeys/p0"))
          return json({ code: "conflict", message: "Operation rejected" }, 409);
        return baseFetch(input);
      }),
    );
    await mount("/settings/security");
    fireEvent.click(
      await screen.findByRole("button", { name: "删除 Device A" }),
    );
    fireEvent.click(await screen.findByRole("button", { name: "确认删除" }));
    expect(await screen.findByText(/操作冲突或凭据不可删除/)).toBeVisible();
    expect(screen.getByText("Device A", { exact: true })).toBeInTheDocument();
    expect(loggedIn).toBe(true);
  });
});
