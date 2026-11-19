import {
  createFileRoute,
  Link,
  Outlet,
  useElementScrollRestoration,
  useNavigate,
  useRouterState,
  useRouter,
} from "@tanstack/react-router";
import { useLayoutEffect, useRef, useState } from "react";
import { AdminLayout, Alert, Button, NavGroup, NavItem } from "@bixlab/mtone";
import {
  ArrowLeft,
  BookOpen,
  FolderKanban,
  House,
  LibraryBig,
  ListTodo,
  MessageSquare,
  Search,
  ShieldCheck,
  Trash2,
} from "lucide-react";
import { NotesNavigationBlocked } from "../features/notes/navigation";
import { AuthGate, authError } from "../features/auth/auth";

export const Route = createFileRoute("/_authenticated")({
  beforeLoad: ({ context, location }) => AuthGate(context, location.href),
  component: Shell,
});
function Shell() {
  const bundle = Route.useRouteContext();
  const navigate = useNavigate();
  const router = useRouter();
  const pathname = useRouterState({
    select: (state) => state.location.pathname.replace(/\/$/, "") || "/",
  });
  const collection = useRouterState({
    select: ({ matches }) => {
      if (matches.some(({ routeId }) => routeId === "/_authenticated/projects/$projectId")) return "/projects";
      if (matches.some(({ routeId }) => routeId === "/_authenticated/tasks/$taskId")) return "/tasks";
      if (matches.some(({ routeId }) => routeId === "/_authenticated/files/$fileId")) return "/files";
      return null;
    },
  });
  // The existing tasks route hosts both the list and the board.
  const wide = /^\/(notes|tasks|chat)(?:\/|$)/.test(pathname);
  const frame = useWorkspaceScrollFrame();
  const [busy, setBusy] = useState(false);
  const blockedLogout = useRef<(() => void) | null>(null);
  const [error, setError] = useState("");
  async function logout() {
    setBusy(true);
    setError("");
    try {
      if (/^\/notes(?:\/|$)/.test(router.state.location.pathname)) {
        try {
          await new Promise<void>((resolve, reject) => {
            blockedLogout.current = () => reject(new Error("无法安全离开笔记，尚未退出登录。请先处理保存错误。"));
            void navigate({ to: "/" }).then(resolve, reject);
          });
        } finally { blockedLogout.current = null; }
        if (/^\/notes(?:\/|$)/.test(router.state.location.pathname)) {
          throw new Error("无法安全离开笔记，尚未退出登录。请先处理保存错误。");
        }
      }
      await bundle.api.raw.POST("/auth/logout");
      await bundle.queryClient.cancelQueries();
      bundle.queryClient.clear();
      await navigate({
        to: "/login",
        search: { redirect: "/" },
        replace: true,
      });
    } catch (cause) {
      setError(authError(cause));
    } finally {
      setBusy(false);
    }
  }
  return (
    <AdminLayout fullHeight>
      <a href="#workspace-content" className="page-link sr-only z-50 bg-surface p-3 focus:not-sr-only focus:fixed focus:top-2 focus:left-2"
        onClick={(event) => {
          const content = document.getElementById("workspace-content");
          if (!content) return;
          event.preventDefault();
          content.focus({ preventScroll: true });
        }}>跳到主要内容</a>
      <AdminLayout.Sider
        logo={<strong>Knoted</strong>}
        collapsedLogo={<strong><span aria-hidden="true">K</span><span className="sr-only">Knoted</span></strong>}
        collapsible
        breakpoint={768}
      >
        <nav aria-label="主导航">
          <NavGroup label="工作空间">
            <NavItem icon={<House size={16} aria-hidden="true" />} active={pathname === "/"}
              onClick={() => void navigate({ to: "/" })}>概览</NavItem>
            <NavItem icon={<BookOpen size={16} aria-hidden="true" />} active={/^\/notes(?:\/|$)/.test(pathname)}
              onClick={() => void navigate({ to: "/notes" })}>笔记</NavItem>
            <NavItem icon={<FolderKanban size={16} aria-hidden="true" />} active={/^\/projects(?:\/|$)/.test(pathname)}
              onClick={() => void navigate({ to: "/projects" })}>项目</NavItem>
            <NavItem icon={<ListTodo size={16} aria-hidden="true" />} active={/^\/tasks(?:\/|$)/.test(pathname)}
              onClick={() => void navigate({ to: "/tasks" })}>事项</NavItem>
            <NavItem icon={<Trash2 size={16} aria-hidden="true" />} active={pathname === "/trash"}
              onClick={() => void navigate({ to: "/trash" })}>回收站</NavItem>
            <NavItem icon={<LibraryBig size={16} aria-hidden="true" />} active={/^\/files(?:\/|$)/.test(pathname)}
              onClick={() => void navigate({ to: "/files" })}>文件</NavItem>
            <NavItem icon={<Search size={16} aria-hidden="true" />} active={pathname === "/search"}
              onClick={() => void navigate({ to: "/search", search: { q: "" } })}>搜索</NavItem>
            <NavItem icon={<MessageSquare size={16} aria-hidden="true" />} active={/^\/chat(?:\/|$)/.test(pathname)}
              onClick={() => void navigate({ to: "/chat" })}>问答</NavItem>
          </NavGroup>
          <NavGroup label="账号">
            <NavItem icon={<ShieldCheck size={16} aria-hidden="true" />} active={pathname === "/settings/security"}
              onClick={() => void navigate({ to: "/settings/security" })}>凭据与会话</NavItem>
          </NavGroup>
        </nav>
      </AdminLayout.Sider>
      <AdminLayout className="min-w-0 flex-1">
        <AdminLayout.Header className="px-3 sm:px-6">
          <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
            {collection && (
              <Link className="page-link inline-flex items-center gap-1" to={collection} resetScroll={collection !== "/files"} activeOptions={{ exact: true }}>
                <ArrowLeft aria-hidden="true" size={16} />
                {{ "/projects": "返回项目", "/tasks": "返回事项", "/files": "返回文件列表" }[collection]}
              </Link>
            )}
            <Button className="ms-auto" loading={busy} onClick={() => void logout()}>退出登录</Button>
          </div>
        </AdminLayout.Header>
        <AdminLayout.Content id="workspace-content" tabIndex={-1} data-workspace-content
          data-scroll-restoration-id="workspace" className="workspace-content bg-canvas p-0">
          <div ref={frame} className={wide ? "workspace-frame workspace-frame--wide" : "workspace-frame"}>
            {error && <div className="mb-4"><Alert tone="error">{error}</Alert></div>}
            <NotesNavigationBlocked.Provider value={() => blockedLogout.current?.()}>
              <Outlet />
            </NotesNavigationBlocked.Provider>
          </div>
        </AdminLayout.Content>
      </AdminLayout>
    </AdminLayout>
  );
}

function useWorkspaceScrollFrame() {
  const frame = useRef<HTMLDivElement>(null);
  const entry = useRouterState({
    select: ({ location, resolvedLocation }) =>
      location.state.__TSR_key === resolvedLocation?.state.__TSR_key ? location.state.__TSR_key : undefined,
  });
  const hasTarget = useRouterState({
    select: ({ location }) => Boolean(location.hash ||
      (location.pathname.startsWith("/chat/") && typeof location.search.message === "string")),
  });
  const savedTop = useElementScrollRestoration({ id: "workspace" })?.scrollY ?? 0;

  useLayoutEffect(() => {
    const body = frame.current;
    const viewport = body?.parentElement;
    if (!entry || !body || !viewport || !savedTop || hasTarget || typeof ResizeObserver === "undefined") return;
    const fits = () => viewport.scrollHeight - viewport.clientHeight >= savedTop;
    if (fits()) return;

    // The router may restore while this entry is still showing a short
    // loading state. Retry once it fits, unless the user takes over first.
    const listeners = new AbortController();
    const observer = new ResizeObserver(() => {
      if (!fits()) return;
      cancel();
      viewport.scrollTop = savedTop;
    });
    function cancel() {
      observer.disconnect();
      listeners.abort();
    }
    observer.observe(body);
    observer.observe(viewport);
    const options = { capture: true, passive: true, signal: listeners.signal };
    for (const event of ["wheel", "pointerdown", "touchstart", "keydown"]) {
      window.addEventListener(event, cancel, options);
    }
    return cancel;
  }, [entry, hasTarget, savedTop]);

  return frame;
}
