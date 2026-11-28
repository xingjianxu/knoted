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
import {
  Alert, AppLayout, Avatar, Breadcrumb, Dialog, NavGroup, NavItem, Topbar,
} from "@bixlab/mtone";
import {
  ArrowLeft,
  Bell,
  BookOpen,
  FolderKanban,
  House,
  LibraryBig,
  ListTodo,
  LogOut,
  Search,
  ShieldCheck,
  Trash2,
  UserRound,
} from "lucide-react";
import { NotesNavigationBlocked } from "../features/notes/navigation";
import { AuthGate, authError } from "../features/auth/auth";

export const Route = createFileRoute("/_authenticated")({
  beforeLoad: ({ context, location, abortController }) => AuthGate(context, location.href, abortController.signal),
  component: Shell,
});

const detailLabels = {
  "/projects": { back: "返回项目", current: "项目详情" },
  "/tasks": { back: "返回事项", current: "事项详情" },
  "/files": { back: "返回文件列表", current: "文件详情" },
};

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
  const notesLayout = /^\/notes(?:\/|$)/.test(pathname);
  const wide = /^\/tasks(?:\/|$)/.test(pathname);
  const frameClass = notesLayout
    ? "workspace-frame workspace-frame--flush"
    : wide
      ? "workspace-frame workspace-frame--wide"
      : "workspace-frame";
  const frame = useWorkspaceScrollFrame();
  const shellRef = useRef<HTMLDivElement>(null);
  const [busy, setBusy] = useState(false);
  const blockedLogout = useRef<(() => void) | null>(null);
  const [error, setError] = useState("");
  const [notificationsOpen, setNotificationsOpen] = useState(false);
  const openSearch = () => void navigate({ to: "/search", search: { q: "" } });
  const openSettings = () => void navigate({ to: "/settings/security" });

  async function logout() {
    if (busy) return;
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
    <>
      <a
        href="#workspace-content"
        className="page-link sr-only z-50 bg-surface p-3 focus:not-sr-only focus:fixed focus:top-2 focus:left-2"
        onClick={(event) => {
          const content = document.getElementById("workspace-content");
          if (!content) return;
          event.preventDefault();
          content.focus({ preventScroll: true });
        }}
      >
        跳到主要内容
      </a>
      <AppLayout
        ref={shellRef}
        fullHeight
        className="workspace-app-layout"
        sidebarWidth={228}
        sidebarCollapsible
        brand={
          <span className="workspace-brand">
            <span className="workspace-brand-mark" aria-hidden="true">K</span>
            <span className="workspace-brand-name">Knoted</span>
          </span>
        }
        navigationLabel="主导航"
        navigation={
          <NavGroup label="工作空间">
            <NavItem
              icon={<House size={16} aria-hidden="true" />}
              active={pathname === "/"}
              onClick={() => void navigate({ to: "/" })}
            >
              概览
            </NavItem>
            <NavItem
              icon={<BookOpen size={16} aria-hidden="true" />}
              active={/^\/notes(?:\/|$)/.test(pathname)}
              onClick={() => void navigate({ to: "/notes" })}
            >
              笔记
            </NavItem>
            <NavItem
              icon={<FolderKanban size={16} aria-hidden="true" />}
              active={/^\/projects(?:\/|$)/.test(pathname)}
              onClick={() => void navigate({ to: "/projects" })}
            >
              项目
            </NavItem>
            <NavItem
              icon={<ListTodo size={16} aria-hidden="true" />}
              active={/^\/tasks(?:\/|$)/.test(pathname)}
              onClick={() => void navigate({ to: "/tasks" })}
            >
              事项
            </NavItem>
            <NavItem
              icon={<LibraryBig size={16} aria-hidden="true" />}
              active={/^\/files(?:\/|$)/.test(pathname)}
              onClick={() => void navigate({ to: "/files" })}
            >
              文件
            </NavItem>
            <NavItem
              icon={<Search size={16} aria-hidden="true" />}
              active={pathname === "/search"}
              onClick={openSearch}
            >
              搜索
            </NavItem>
            <NavItem
              icon={<Trash2 size={16} aria-hidden="true" />}
              active={pathname === "/trash"}
              onClick={() => void navigate({ to: "/trash" })}
            >
              回收站
            </NavItem>
          </NavGroup>
        }
        account={{
          name: "个人账号",
          description: "Knoted 工作空间",
          avatar: <Avatar size={28} icon={<UserRound size={15} />} />,
          menuLabel: "账号菜单",
          items: [
            {
              key: "security",
              label: "凭据与会话",
              icon: <ShieldCheck size={15} aria-hidden="true" />,
              onSelect: openSettings,
            },
            {
              key: "notifications",
              label: "通知",
              icon: <Bell size={15} aria-hidden="true" />,
              onSelect: () => setNotificationsOpen(true),
            },
            { type: "divider", key: "before-logout" },
            {
              key: "logout",
              label: busy ? "正在退出…" : "退出登录",
              icon: <LogOut size={15} aria-hidden="true" />,
              danger: true,
              disabled: busy,
              onSelect: () => void logout(),
            },
          ],
        }}
        contentAs="div"
      >
        <main
          id="workspace-content"
          tabIndex={-1}
          data-workspace-content
          data-scroll-restoration-id="workspace"
          className={notesLayout ? "workspace-content workspace-content--flush" : "workspace-content bg-canvas"}
        >
          {collection && (
            <Topbar
              as="div"
              className="workspace-detail-topbar"
              breadcrumb={
                <Breadcrumb
                  aria-label="当前位置"
                  className="[&>ol>li:first-child]:shrink-0"
                  items={[
                    {
                      label: (
                        <Link
                          className="page-link inline-flex items-center gap-1"
                          to={collection}
                          resetScroll={collection !== "/files"}
                          activeOptions={{ exact: true }}
                        >
                          <ArrowLeft aria-hidden="true" size={16} />
                          {detailLabels[collection].back}
                        </Link>
                      ),
                    },
                    { label: detailLabels[collection].current },
                  ]}
                />
              }
            />
          )}
          <div ref={frame} className={frameClass}>
            {error && (
              <div className={notesLayout ? "shrink-0 px-3 pt-3 sm:px-6 sm:pt-4" : "mb-4"}>
                <Alert tone="error">{error}</Alert>
              </div>
            )}
            <NotesNavigationBlocked.Provider value={() => blockedLogout.current?.()}>
              <Outlet />
            </NotesNavigationBlocked.Provider>
          </div>
        </main>
      </AppLayout>
      <Dialog
        title="通知"
        open={notificationsOpen}
        onOpenChange={(open) => {
          setNotificationsOpen(open);
          if (!open) {
            requestAnimationFrame(() => {
              shellRef.current?.querySelector<HTMLElement>(".mt-app-layout-account")?.focus();
            });
          }
        }}
      >
        <p className="text-muted">暂无通知</p>
      </Dialog>
    </>
  );
}

function useWorkspaceScrollFrame() {
  const frame = useRef<HTMLDivElement>(null);
  const entry = useRouterState({
    select: ({ location, resolvedLocation }) =>
      location.state.__TSR_key === resolvedLocation?.state.__TSR_key ? location.state.__TSR_key : undefined,
  });
  const hasTarget = useRouterState({
    select: ({ location }) => Boolean(location.hash),
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
