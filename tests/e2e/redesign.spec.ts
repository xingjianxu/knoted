import { expect, test, type Locator, type Page } from "@playwright/test";
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import type { components } from "../../packages/api-client/src/generated/schema";

const id = (number: number) => `00000000-0000-4000-8000-${String(number).padStart(12, "0")}`;
const now = "2026-11-19T00:00:00Z";
const timestamps = { created_at: now, updated_at: now };
const list = <T,>(items: T[], next_cursor: string | null = null) => ({ items, next_cursor });
const project = { id: id(4), title: "调研项目", description: "整理公开材料与调研记录。", version: 1, archived_at: null, deleted_at: null, ...timestamps };
const task = { id: id(5), title: "校对调研报告", description: "复核数据和引用。", project_id: project.id, parent_id: null, version: 1,
  status: "todo", priority: "high", start: null, due: { date: "2026-11-19", time: "17:30", timezone: "Asia/Shanghai" }, archived_at: null, deleted_at: null, ...timestamps };
const document = { blocks: Array.from({ length: 14 }, (_, i) => ({
  id: id(i + 100), type: "paragraph", content: [{ type: "text", text: `调研记录第 ${i + 1} 段，记录公开材料、复核过程及后续行动。`, styles: {} }],
})) };
const note = { id: id(6), title: "调研手记", document, content_version: 2, format_version: 1,
  parent_id: null, position: 0, deleted_at: null, ...timestamps };
const file = { id: id(7), name: "调研结果.txt", relative_path: "资料/项目/调研结果.txt", note: "校对后发布。", size: 280,
  mime: "text/plain", tag_ids: [], sha256: "b".repeat(64), version: 1, content_version: 1, status: "present", last_seen_at: now, ...timestamps };
const chat = { id: id(8), title: "资料问答", provider: "local", version: 1, ...timestamps };
const current = { id: id(2), user_id: id(3), current: true, created_at: now, last_seen_at: now,
  idle_expires_at: "2026-11-30T00:00:00Z", absolute_expires_at: "2026-12-19T00:00:00Z" };
const messages = Array.from({ length: 8 }, (_, i) => ({
  id: id(60 + i), session_id: chat.id, role: i % 2 === 0 ? "user" : "assistant", run_id: id(80 + Math.floor(i / 2)),
  body: i % 2 === 0 ? `调研材料第 ${i + 1} 条记录了什么？` : `这是第 ${i + 1} 条材料的摘要。资料包括引用、数据和复核信息。`,
  incomplete: false, created_at: now,
}));
const target = messages.at(-1)!;
const fixtureFileText = "合成文本文件用于核对鉴权预览界面。";

async function fixtures(page: Page, authenticated = true) {
  const fileItems = Array.from({ length: 20 }, (_, i) => ({ ...file, id: id(200 + i), name: `资料 ${i + 1}.txt` }));
  let restored = false;
  let activeSession = authenticated;
  let currentTask = task;
  let failTrashRefresh = false;
  await page.route("**/api/v1/**", (route) => {
    const { pathname, searchParams } = new URL(route.request().url());
    const json = (data: unknown) => route.fulfill({ json: data });
    switch (pathname.replace("/api/v1", "")) {
      case "/auth/session": return activeSession ? json({ session: current, csrf_token: "synthetic-redesign-fixture" }) :
        route.fulfill({ status: 401, json: { code: "unauthorized", message: "未登录" } });
      case "/auth/passkeys": return json(list([{ id: id(9), name: "验收凭据", created_at: now, last_used_at: now }]));
      case "/auth/sessions": return json(list([current]));
      case `/auth/sessions/${current.id}`:
        expect(route.request().method()).toBe("DELETE");
        activeSession = false;
        return route.fulfill({ status: 204 });
      case "/dashboard": return json({
        counts: { notes: 4, tasks_open: 7, tasks_due_today: 2, projects_active: 3, files: 21 },
        tasks_by_status: { todo: 3, in_progress: 4, done: 5, cancelled: 1 },
        recent_notes: [note], recent_tasks: [], due_tasks: [task, {
          ...task, id: id(14), title: "补全引用", due: { ...task.due, date: "2020-11-18" },
        }], job_failures: 1, indexing_enabled: false,
      });
      case "/notes": return json(list([note, { ...note, id: id(15), title: "后续问题", parent_id: note.id }]));
      case `/notes/${note.id}`:
        if (route.request().method() === "PATCH") return route.fulfill({
          status: 409, json: { code: "version_conflict", message: "版本冲突", current_version: 3 },
        });
        return json(note);
      case `/notes/${note.id}/snapshots`:
        if (route.request().method() === "POST") return json({ ...note, note_id: note.id, snapshot_id: id(51), reason: "flush" });
        return json(list([{
          ...note, note_id: note.id, snapshot_id: id(50), title: "历史调研手记", content_version: 1, reason: "flush",
        }]));
      case "/projects": return json(list([project]));
      case `/projects/${project.id}`: return json(project);
      case "/tasks":
        if (searchParams.has("parent_id")) return json(list([]));
        if (searchParams.get("limit") === "50" && searchParams.get("project_id") === project.id && !searchParams.has("status")) {
          return searchParams.has("cursor") ? json(list([currentTask])) : json(list([{ ...task, id: id(16), title: "第一页事项" }], "tasks-page-2"));
        }
        return json(list([currentTask]));
      case `/tasks/${task.id}`:
        if (route.request().method() === "PATCH") {
          const payload = route.request().postDataJSON();
          expect(payload).toMatchObject({ status: "in_progress", expected_version: currentTask.version });
          currentTask = { ...currentTask, status: payload.status, version: currentTask.version + 1 };
        }
        return json(currentTask);
      case `/tasks/${task.id}/activities`: return json(list([]));
      case "/files": return json(list([...fileItems, file]));
      case `/files/${file.id}`: return json(file);
      case `/files/${file.id}/content`: return route.fulfill({
        body: fixtureFileText, contentType: "text/plain; charset=utf-8", headers: { etag: '"1"' },
      });
      case "/jobs": return json(list([{
        id: id(12), kind: "file.extract", state: "failed", attempts: 1, max_attempts: 3, error: "unsupported_format",
        started_at: now, finished_at: now, ...timestamps,
      }]));
      case "/search": return json(list([{
        id: target.id, session_id: chat.id, entity_type: "chat", title: "会话中的调研材料",
        snippet: "调研资料包括引用、数据和复核信息。", path: null, score: null, updated_at: now,
      }]));
      case "/chat/sessions": return json(list([chat]));
      case `/chat/sessions/${chat.id}`: return json({ ...chat, messages });
      case "/trash": {
        if (failTrashRefresh) {
          return route.fulfill({ status: 503, json: { code: "dependency_unavailable", message: "刷新暂时失败" } });
        }
        return json(list([
          ...(!restored ? [{ id: id(10), title: "旧版手记", entity_type: "note", parent_id: id(11), version: 1,
            deleted_at: now, can_restore: false, reason: "原父笔记已删除，可恢复到根目录。" }] : []),
          { id: id(13), title: "待查证材料", entity_type: "task", parent_id: null, version: 1,
            deleted_at: now, can_restore: false, reason: "父事项已删除。" },
        ]));
      }
      case `/notes/${id(10)}/restore`: {
        if (route.request().method() !== "POST" || route.request().postDataJSON().restore_parent?.parent_id !== null) {
          return route.fulfill({ status: 422, json: { code: "invalid_request", message: "请选择根目录" } });
        }
        restored = true;
        failTrashRefresh = true;
        return json({ ...note, id: id(10) });
      }
      case "/tags":
      case "/links":
      case `/entities/note/${note.id}/tags`:
      case `/entities/note/${id(15)}/tags`:
      case `/entities/project/${project.id}/tags`:
        return json(list([]));
      default: throw new Error(`Unexpected fixture request: ${route.request().method()} ${pathname}`);
    }
  });
}

type RouteSample = {
  name: string;
  path: string;
  ready: (page: Page) => Locator;
  section?: string;
};
const heading = (name: string | RegExp) => (page: Page) => page.getByRole("heading", { name, exact: true }).first();
const routeSamples: RouteSample[] = [
  { name: "login", path: "/login", ready: heading("登录") },
  { name: "bootstrap", path: "/bootstrap", ready: heading("首次注册") },
  { name: "recovery", path: "/recovery", ready: heading("恢复账号") },
  { name: "dashboard", path: "/", ready: heading("工作概览"), section: "概览" },
  { name: "notes", path: "/notes", ready: page => page.getByRole("textbox", { name: "笔记标题" }), section: "笔记" },
  { name: "note", path: `/notes/${note.id}`, ready: page => page.getByRole("textbox", { name: "笔记标题" }), section: "笔记" },
  { name: "projects", path: "/projects", ready: heading("项目"), section: "项目" },
  { name: "project", path: `/projects/${project.id}`, ready: heading(project.title), section: "项目" },
  { name: "tasks", path: "/tasks", ready: heading("事项"), section: "事项" },
  { name: "task", path: `/tasks/${task.id}`, ready: heading(/^校对调研报告/), section: "事项" },
  { name: "files", path: "/files", ready: heading("文件"), section: "文件" },
  { name: "file", path: `/files/${file.id}`, ready: heading(file.name), section: "文件" },
  { name: "jobs", path: "/files/jobs", ready: heading("索引任务"), section: "文件" },
  { name: "search", path: "/search?q=调研", ready: page => page.getByRole("link", { name: "会话中的调研材料" }), section: "搜索" },
  { name: "chat", path: "/chat", ready: page => page.getByRole("link", { name: chat.title }), section: "问答" },
  { name: "thread", path: `/chat/${chat.id}`, ready: heading(chat.title), section: "问答" },
  { name: "trash", path: "/trash", ready: heading("回收站"), section: "回收站" },
  { name: "security", path: "/settings/security", ready: heading("凭据与会话"), section: "凭据与会话" },
  { name: "404", path: "/missing-route", ready: heading("页面不存在") },
];

const baseline = process.env.KNOTED_REDESIGN_BASELINE === "1";
const capture = process.env.KNOTED_REDESIGN_CAPTURE;

for (const width of [1440, 768, 390, 320]) {
  for (const sample of routeSamples) {
    test(`矩阵 ${width}px ${sample.name}`, async ({ page }) => {
      test.skip(baseline && width !== 1440 && width !== 390, "基线仅记录桌面与手机对照");
      await page.setViewportSize({ width, height: width > 768 ? 900 : 844 });
      await fixtures(page, !(["login", "bootstrap", "recovery"].includes(sample.name)));
      const errors: Error[] = [];
      page.on("pageerror", error => errors.push(error));
      await page.goto(`http://127.0.0.1:4178${sample.path}`);
      await expect(sample.ready(page)).toBeVisible();
      if (sample.section) {
        await expect(page.getByRole("navigation", { name: "主导航" }).getByRole("button", { name: sample.section }))
          .toHaveAttribute("aria-current", "page");
      }
      if (!baseline) {
        await expect(sample.ready(page)).toBeInViewport();
        expect(await page.getByRole("main").evaluate(main => main.scrollWidth - main.clientWidth)).toBeLessThanOrEqual(1);
        expect(errors).toEqual([]);
      }
      if (capture && (width === 1440 || width === 390)) {
        await page.evaluate(() => document.fonts.ready.then(() => {}));
        await page.screenshot({ path: `${capture}/${width}-${sample.name}.png`, animations: "disabled" });
      }
    });
  }
}

test("窄屏文件列表打开详情，预览源码并关闭回焦，再返回列表", async ({ page }) => {
  test.skip(baseline, "改版后的布局回归");
  await page.setViewportSize({ width: 390, height: 844 });
  await fixtures(page);
  await page.goto("http://127.0.0.1:4178/files");
  const fileRow = page.getByRole("button", { name: `打开文件 ${file.name}` });
  await fileRow.click();
  await expect(page.getByRole("heading", { name: file.name, level: 1 })).toBeInViewport();
  const preview = page.getByRole("button", { name: "预览文件" });
  await preview.focus();
  await page.keyboard.press("Enter");
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByText(fixtureFileText)).toBeVisible();
  await expect(dialog.getByRole("button", { name: "下载原文件" })).toBeInViewport();
  if (capture) await page.screenshot({ path: `${capture}/390-file-preview.png`, animations: "disabled" });
  await page.keyboard.press("Escape");
  await expect(preview).toBeFocused();
  await page.getByRole("link", { name: "返回文件列表" }).click();
  await expect(fileRow).toBeFocused();
  await expect(fileRow).toBeInViewport();
});

test("回收站确认关闭回焦，恢复成功但刷新失败时已恢复行移除", async ({ page }) => {
  test.skip(baseline, "改版后的布局回归");
  await page.setViewportSize({ width: 390, height: 844 });
  await fixtures(page);
  await page.goto("http://127.0.0.1:4178/trash");
  const trigger = page.getByRole("button", { name: "恢复 旧版手记" });
  await trigger.focus();
  await page.keyboard.press("Enter");
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByText(/恢复到根目录/)).toBeVisible();
  if (capture) await page.screenshot({ path: `${capture}/390-trash-restore.png`, animations: "disabled" });
  await page.keyboard.press("Escape");
  await expect(trigger).toBeFocused();
  await page.keyboard.press("Enter");
  await dialog.getByRole("button", { name: "确认恢复" }).click();
  await expect(page.getByRole("heading", { name: "回收站", level: 1 })).toBeFocused();
  await expect(page.getByRole("status").getByText("已恢复 旧版手记")).toBeVisible();
  await expect(page.getByRole("alert").getByText("列表刷新失败")).toBeVisible();
  await expect(trigger).toHaveCount(0);
});

test("概览先读上海到期状态，打开事项后返回默认清单", async ({ page }) => {
  test.skip(baseline, "改版后的布局回归");
  await page.setViewportSize({ width: 390, height: 844 });
  await page.clock.setFixedTime(new Date("2026-11-19T06:00:00Z"));
  await fixtures(page);
  await page.goto("http://127.0.0.1:4178/");
  const due = page.getByRole("list", { name: "到期事项" });
  const first = due.getByRole("listitem").filter({ hasText: task.title });
  await expect(first).toContainText("今日到期");
  await expect(due.getByRole("listitem").filter({ hasText: "补全引用" })).toContainText("已逾期");
  await first.getByRole("button", { name: `打开事项 ${task.title}` }).click();
  await expect(page.getByRole("heading", { name: task.title, level: 1 })).toBeInViewport();
  await expect(page.getByRole("heading", { name: "跟进时间线" })).toBeVisible();
  await page.getByRole("link", { name: "返回事项" }).click();
  await expect(page.getByRole("radio", { name: "列表视图" })).toBeChecked();
});

test("从筛选后的看板第二页查看事项，返回后使用键盘移列", async ({ page }) => {
  test.skip(baseline, "改版后的布局回归");
  await page.setViewportSize({ width: 1440, height: 900 });
  await fixtures(page);
  await page.goto("http://127.0.0.1:4178/tasks");
  await page.getByRole("radio", { name: "看板视图" }).click();
  await page.getByRole("combobox", { name: "项目筛选" }).click();
  await page.getByRole("option", { name: project.title }).click();
  await page.getByRole("button", { name: "下一页事项" }).click();
  await page.getByRole("link", { name: task.title }).click();
  await page.getByRole("link", { name: "返回事项" }).click();
  await expect(page.getByRole("radio", { name: "看板视图" })).toBeChecked();
  await expect(page.getByRole("combobox", { name: "项目筛选" })).toHaveText(project.title);
  await expect(page.getByRole("button", { name: "上一页事项" })).toBeEnabled();
  const card = page.getByRole("listitem").filter({ has: page.getByRole("link", { name: task.title }) });
  await expect(card).toHaveAttribute("aria-disabled", "false");
  await card.focus();
  await page.keyboard.press("Alt+ArrowRight");
  const moved = page.getByRole("list", { name: "进行中事项" }).getByRole("listitem");
  await expect(moved).toContainText(task.title);
  await expect(moved).toBeFocused();
  if (capture) await page.screenshot({ path: `${capture}/1440-board.png`, animations: "disabled" });
});

test("笔记历史立即可读，保存冲突后离开及退出均保留草稿", async ({ page }) => {
  test.skip(baseline, "改版后的布局回归");
  await page.setViewportSize({ width: 390, height: 844 });
  await fixtures(page);
  await page.goto(`http://127.0.0.1:4178/notes/${note.id}`);
  const editor = page.getByRole("textbox", { name: "笔记标题" });
  const history = page.getByRole("button", { name: "历史版本" });
  await history.click();
  const historyDivider = page.getByRole("region", { name: "当前笔记" }).locator(".mt-container > hr.mt-divider");
  const historySection = page.getByRole("region", { name: "笔记历史" });
  await expect(historyDivider).toBeVisible();
  const [historyDividerBox, historySectionBox] = await Promise.all([
    historyDivider.boundingBox(),
    historySection.boundingBox(),
  ]);
  expect(historyDividerBox).not.toBeNull();
  expect(historySectionBox).not.toBeNull();
  expect(historySectionBox!.y - (historyDividerBox!.y + historyDividerBox!.height)).toBeGreaterThanOrEqual(8);
  await page.getByRole("button", { name: /预览 历史调研手记/ }).click();
  await expect(page.getByRole("heading", { name: "历史调研手记" })).toBeInViewport();
  const preview = page.getByRole("region", { name: "历史预览" });
  await expect(preview).toContainText("调研记录第 1 段");
  await expect(preview.locator('[contenteditable="true"]')).toHaveCount(0);
  if (capture) await page.screenshot({ path: `${capture}/390-note-history.png`, animations: "disabled" });
  await page.getByRole("button", { name: "关闭历史" }).click();
  await expect(history).toBeFocused();
  await expect(page.getByRole("textbox", { name: "笔记编辑器" })).toBeEditable();
  await editor.fill("当前草稿不能丢失");
  await page.getByRole("button", { name: "更多操作" }).click();
  await page.getByRole("menuitem", { name: "退出登录" }).click();
  await expect(editor).toHaveValue("当前草稿不能丢失");
  await expect(page.getByRole("button", { name: "版本与冲突处理" })).toBeVisible();
  await page.getByRole("button", { name: "目录", exact: true }).click();
  const directory = page.getByRole("dialog", { name: "笔记目录" });
  await expect(directory.getByRole("tree")).toBeVisible();
  if (capture) await page.screenshot({ path: `${capture}/390-note-directory.png`, animations: "disabled" });
  await directory.getByRole("button", { name: "关闭" }).click();
  await page.getByRole("navigation", { name: "主导航" }).getByRole("button", { name: "项目" }).click();
  await expect(page).toHaveURL(new RegExp(`/notes/${note.id}$`));
  await expect(editor).toHaveValue("当前草稿不能丢失");
});

test("安全页撤销当前会话，旧保护页重新打开仍需登录", async ({ page }) => {
  test.skip(baseline, "改版后的布局回归");
  await page.setViewportSize({ width: 390, height: 844 });
  await fixtures(page);
  await page.goto("http://127.0.0.1:4178/settings/security");
  await page.getByRole("button", { name: "撤销当前会话" }).click();
  await page.getByRole("alertdialog").getByRole("button", { name: "确认撤销" }).click();
  await expect(page.getByRole("heading", { name: "登录" })).toBeVisible();
  await page.goto("http://127.0.0.1:4178/settings/security");
  await expect(page.getByRole("heading", { name: "登录" })).toBeVisible();
});

test("200% 放大下正文仍可在当前工作区内阅读", async ({ page }) => {
  test.skip(baseline, "改版后的布局回归");
  await page.setViewportSize({ width: 768, height: 900 });
  await fixtures(page);
  await page.goto(`http://127.0.0.1:4178/files/${file.id}`);
  await expect(page.getByRole("heading", { name: file.name, level: 1 })).toBeVisible();
  await page.evaluate(() => { document.documentElement.style.zoom = "2"; });
  await expect(page.getByRole("button", { name: "预览文件" })).toBeInViewport();
  expect(await page.getByRole("main").evaluate(main => main.scrollWidth - main.clientWidth)).toBeLessThanOrEqual(1);
});

test("静态图片在 Chromium 中实际解码，关闭后撤销对象 URL", async ({ page }) => {
  test.skip(baseline, "改版后的预览回归");
  const body = await readFile(resolve(__dirname, "../../apps/web/tests/fixtures/t50/chinese.png"));
  await fixtures(page);
  await page.route(`**/api/v1/files/${file.id}`, route => route.fulfill({
    json: { ...file, name: "调研插图.png", mime: "image/png", size: body.length },
  }));
  await page.route(`**/api/v1/files/${file.id}/content`, route => route.fulfill({
    body, contentType: "image/png", headers: { etag: '"1"' },
  }));
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(`http://127.0.0.1:4178/files/${file.id}`);
  const preview = page.getByRole("button", { name: "预览文件" });
  await preview.click();
  const image = page.getByRole("img", { name: "调研插图.png" });
  await expect.poll(() => image.evaluate((element: HTMLImageElement) => element.naturalWidth)).toBeGreaterThan(0);
  const objectUrl = await image.getAttribute("src");
  expect(objectUrl).toMatch(/^blob:/);
  if (capture) await page.screenshot({ path: `${capture}/390-image-preview.png`, animations: "disabled" });
  await page.getByRole("button", { name: "关闭预览" }).click();
  await expect(preview).toBeFocused();
  expect(await page.evaluate(url => fetch(url!).then(() => true, () => false), objectUrl)).toBe(false);
});

test("笔记目录选中新笔记和切换到宽屏后焦点仍在可见界面", async ({ page }) => {
  test.skip(baseline, "改版后的布局回归");
  await fixtures(page);
  const child = { ...note, id: id(15), title: "后续问题", parent_id: note.id };
  await page.route(`**/api/v1/notes/${child.id}`, route => route.fulfill({ json: child }));
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(`http://127.0.0.1:4178/notes/${note.id}`);
  await expect(page.getByRole("complementary", { name: "笔记目录" })).toBeHidden();
  const opener = page.getByRole("button", { name: "目录", exact: true });
  await opener.click();
  const dialog = page.getByRole("dialog", { name: "笔记目录" });
  const root = dialog.getByRole("treeitem", { name: /调研手记/ });
  await root.focus();
  await page.keyboard.press("ArrowRight");
  await dialog.getByRole("treeitem", { name: /后续问题/ }).click();
  await expect(page.getByRole("textbox", { name: "笔记标题" })).toHaveValue(child.title);
  await expect(dialog).toBeHidden();
  await expect(opener).toBeFocused();
  await opener.click();
  await page.setViewportSize({ width: 1440, height: 900 });
  await expect(dialog).toBeHidden();
  await expect(page.getByRole("heading", { name: "笔记目录" })).toBeFocused();
  const sidebarBox = await page.getByRole("complementary", { name: "笔记目录" }).boundingBox();
  expect(sidebarBox?.width ?? 0).toBeGreaterThan(150);
});

test("笔记页使用 Splitter 左右分栏，默认展示树根首篇笔记并支持拖拽调节", async ({ page }) => {
  test.skip(baseline, "改版后的布局回归");
  await fixtures(page);
  const child = { ...note, id: id(15), title: "后续问题", parent_id: note.id, document: { blocks: [{ id: id(150), type: "paragraph", content: [{ type: "text", text: "子笔记正文内容", styles: {} }] }] } };
  await page.route(`**/api/v1/notes/${child.id}`, route => route.fulfill({ json: child }));
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("http://127.0.0.1:4178/notes");

  const sidebar = page.getByRole("complementary", { name: "笔记目录" });
  const current = page.getByRole("region", { name: "当前笔记" });
  const rootItem = sidebar.getByRole("treeitem", { name: /调研手记/ });
  await expect(rootItem).toHaveAttribute("aria-selected", "true");
  await expect(page.getByRole("textbox", { name: "笔记标题" })).toHaveValue(note.title);
  await expect(current).toContainText("调研记录第 1 段");

  const [beforeSidebar, beforeContent] = await Promise.all([sidebar.boundingBox(), current.boundingBox()]);
  expect(beforeSidebar).toBeTruthy();
  expect(beforeContent).toBeTruthy();
  expect(beforeSidebar!.x + beforeSidebar!.width).toBeLessThanOrEqual(beforeContent!.x + 1);
  expect(beforeSidebar!.height).toBeGreaterThan(700);
  expect(beforeContent!.height).toBeGreaterThan(700);

  const separator = page.getByRole("separator", { name: "调整分栏 1" });
  const sepBox = await separator.boundingBox();
  expect(sepBox).toBeTruthy();
  const startX = sepBox!.x + sepBox!.width / 2;
  const startY = sepBox!.y + sepBox!.height / 2;
  await page.mouse.move(startX, startY);
  await page.mouse.down();
  await page.mouse.move(startX + 80, startY, { steps: 5 });
  await page.mouse.up();
  await expect.poll(async () => (await sidebar.boundingBox())?.width ?? 0).toBeGreaterThan(beforeSidebar!.width + 40);
  expect(await page.getByRole("main").evaluate(main => main.scrollWidth - main.clientWidth)).toBeLessThanOrEqual(1);

  await rootItem.focus();
  await page.keyboard.press("ArrowRight");
  await sidebar.getByRole("treeitem", { name: /后续问题/ }).click();
  await expect(page).toHaveURL(new RegExp(`/notes/${child.id}$`));
  await expect(page.getByRole("textbox", { name: "笔记标题" })).toHaveValue(child.title);
  await expect(current).toContainText("子笔记正文内容");
});

test("笔记标题和编辑器共用居中的 Container，并随分栏和窄屏收缩", async ({ page }) => {
  test.skip(baseline, "改版后的笔记编辑器布局回归");
  await fixtures(page);
  await page.route(`**/api/v1/notes/${note.id}`, route =>
    route.fulfill({ json: { ...note, document: { blocks: document.blocks.slice(0, 1) } } }),
  );
  await page.setViewportSize({ width: 1920, height: 900 });
  await page.goto(`http://127.0.0.1:4178/notes/${note.id}`);

  const pane = page.getByRole("region", { name: "当前笔记" });
  const paneBar = pane.locator(".notes-pane-bar");
  const container = pane.locator(".mt-container");
  const heading = container.locator(".notes-document-heading");
  const title = container.getByRole("textbox", { name: "笔记标题" });
  const blockEditor = container.locator(".mt-block-editor");
  const editor = container.getByRole("textbox", { name: "笔记编辑器" });
  const sidebar = page.getByRole("complementary", { name: "笔记目录" });
  const expectFilled = async () => {
    await expect(title).toHaveValue(note.title);
    await expect(editor).toBeVisible();
    await expect(container.getByRole("separator")).toHaveCount(0);

    const spacing = await title.evaluate(el => {
      const inputStyle = getComputedStyle(el);
      const wrapStyle = el.parentElement ? getComputedStyle(el.parentElement) : null;
      const headingEl = el.closest(".notes-document-heading");
      const headingStyle = headingEl ? getComputedStyle(headingEl) : null;
      return {
        inputMarginBottom: inputStyle.marginBottom,
        inputPaddingTop: inputStyle.paddingTop,
        wrapMarginBottom: wrapStyle?.marginBottom,
        headingPaddingTop: headingStyle?.paddingTop,
      };
    });
    expect(spacing.inputMarginBottom).toBe("0px");
    expect(spacing.inputPaddingTop).toBe("0px");
    expect(spacing.wrapMarginBottom).toBe("0px");
    expect(spacing.headingPaddingTop).toBe("0px");

    const [paneBox, barBox, containerBox, headingBox, titleBox, blockEditorBox] = await Promise.all([
      pane.boundingBox(),
      paneBar.boundingBox(),
      container.boundingBox(),
      heading.boundingBox(),
      title.boundingBox(),
      blockEditor.boundingBox(),
    ]);
    expect(paneBox).not.toBeNull();
    expect(barBox).not.toBeNull();
    expect(containerBox).not.toBeNull();
    expect(headingBox).not.toBeNull();
    expect(titleBox).not.toBeNull();
    expect(blockEditorBox).not.toBeNull();

    expect(Math.abs(containerBox!.width - Math.min(960, paneBox!.width))).toBeLessThanOrEqual(2);
    expect(Math.abs(containerBox!.x + containerBox!.width / 2 - (paneBox!.x + paneBox!.width / 2))).toBeLessThanOrEqual(2);
    expect(Math.abs(blockEditorBox!.x - (containerBox!.x + 16))).toBeLessThanOrEqual(2);
    expect(Math.abs(blockEditorBox!.width - (containerBox!.width - 32))).toBeLessThanOrEqual(2);
    expect(Math.abs(titleBox!.x - blockEditorBox!.x)).toBeLessThanOrEqual(2);

    const topGap = titleBox!.y - (barBox!.y + barBox!.height);
    expect(topGap).toBeGreaterThanOrEqual(8);
    expect(topGap).toBeLessThanOrEqual(18);

    const titleToEditorGap = blockEditorBox!.y - (headingBox!.y + headingBox!.height);
    expect(titleToEditorGap).toBeGreaterThanOrEqual(6);
    expect(titleToEditorGap).toBeLessThanOrEqual(14);

    const bottomGap = paneBox!.y + paneBox!.height - (blockEditorBox!.y + blockEditorBox!.height);
    expect(bottomGap).toBeGreaterThanOrEqual(12);
    expect(bottomGap).toBeLessThanOrEqual(24);
  };

  await expectFilled();
  expect((await container.boundingBox())!.x).toBeGreaterThan((await pane.boundingBox())!.x);
  await expect(sidebar).toBeVisible();
  await expect(container.getByRole("tree")).toHaveCount(0);
  await expect(container.getByRole("button", { name: "历史版本" })).toHaveCount(0);

  await page.setViewportSize({ width: 1280, height: 900 });
  await expectFilled();
  const before = (await blockEditor.boundingBox())!.width;
  await page.getByRole("separator", { name: "调整分栏 1" }).press("ArrowRight");
  await expect.poll(async () => (await blockEditor.boundingBox())?.width ?? before).toBeLessThan(before);
  await expectFilled();

  await page.setViewportSize({ width: 390, height: 844 });
  await expect(page.getByRole("button", { name: "目录", exact: true })).toBeVisible();
  await expectFilled();
});

test("长笔记在正文区滚动时保留工具栏", async ({ page }) => {
  test.skip(baseline, "改版后的笔记编辑器布局回归");
  await fixtures(page);
  await page.setViewportSize({ width: 390, height: 540 });
  await page.goto(`http://127.0.0.1:4178/notes/${note.id}`);

  const scroll = page.locator(".notes-body-scroll");
  const blockEditor = scroll.locator(".mt-block-editor");
  const editor = page.getByRole("textbox", { name: "笔记编辑器" });
  const lastParagraph = editor.getByText(/^调研记录第 14 段/);
  await expect(lastParagraph).toBeAttached();
  await expect(lastParagraph).not.toBeInViewport();
  await lastParagraph.scrollIntoViewIfNeeded();
  await expect(lastParagraph).toBeInViewport();
  await expect(page.getByRole("button", { name: "历史版本" })).toBeInViewport();

  await scroll.evaluate(el => { el.scrollTop = el.scrollHeight; });
  const [scrollBox, blockEditorBox] = await Promise.all([scroll.boundingBox(), blockEditor.boundingBox()]);
  expect(scrollBox).not.toBeNull();
  expect(blockEditorBox).not.toBeNull();
  const bottomGap = scrollBox!.y + scrollBox!.height - (blockEditorBox!.y + blockEditorBox!.height);
  expect(bottomGap).toBeGreaterThanOrEqual(12);
  expect(bottomGap).toBeLessThanOrEqual(24);
});

test("笔记编辑器消费 mtone-blockeditor：斜杠菜单、图片上传资产映射与不支持块提示", async ({ page }) => {
  test.skip(baseline, "改版后的编辑器回归");
  const uploadedAssetId = id(901);
  const pngBytes = await readFile(resolve(__dirname, "../../apps/web/tests/fixtures/t50/chinese.png"));
  const patches: unknown[] = [];
  let version = 2;

  await fixtures(page);
  await page.route(`**/api/v1/notes/${note.id}`, route => {
    if (route.request().method() === "PATCH") {
      const body = route.request().postDataJSON();
      patches.push(body);
      version += 1;
      return route.fulfill({ json: { ...note, ...body, content_version: version } });
    }
    return route.fulfill({
      json: {
        ...note,
        content_version: version,
        document: { blocks: [{ id: id(100), type: "paragraph", content: [{ type: "text", text: "初始内容" }] }] },
      },
    });
  });
  await page.route(`**/api/v1/notes/${note.id}/snapshots`, route => {
    if (route.request().method() === "POST") {
      return route.fulfill({ status: 201, json: { ...note, note_id: note.id, snapshot_id: id(51), content_version: version, reason: "flush" } });
    }
    return route.fallback();
  });
  let assetUploads = 0;
  await page.route("**/api/v1/assets", route => {
    expect(route.request().method()).toBe("POST");
    assetUploads += 1;
    return route.fulfill({
      status: 201,
      json: { id: uploadedAssetId, mime: "image/png", size: pngBytes.length, sha256: "a".repeat(64), state: "ready", created_at: now },
    });
  });
  await page.route(`**/api/v1/assets/${uploadedAssetId}/content`, route => route.fulfill({
    body: pngBytes,
    contentType: "image/png",
  }));

  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(`http://127.0.0.1:4178/notes/${note.id}`);

  const editor = page.getByRole("textbox", { name: "笔记编辑器" });
  await expect(editor).toBeVisible();
  await expect(page.locator(".mt-block-editor")).toBeVisible();
  await expect(editor).toContainText("初始内容");

  // Type a new line and use the built-in slash menu to insert a heading.
  await editor.click();
  await page.keyboard.press("End");
  await page.keyboard.press("Enter");
  await page.keyboard.type("/");
  const slashList = page.getByRole("listbox", { name: "插入块" });
  await expect(slashList).toBeVisible();
  await slashList.getByRole("option", { name: "一级标题" }).click();
  await page.keyboard.type("真实浏览器标题");

  // Insert an uploaded image via built-in slash menu + ImageForm uploadImage callback.
  await page.keyboard.press("Enter");
  await page.keyboard.type("/");
  await expect(slashList).toBeVisible();
  await slashList.getByRole("option", { name: "图片" }).click();
  const imageSettings = page.getByRole("group", { name: "图片设置" });
  await expect(imageSettings).toBeVisible();
  await imageSettings.getByLabel("本地文件").setInputFiles({
    name: "chinese.png",
    mimeType: "image/png",
    buffer: pngBytes,
  });
  await expect(imageSettings.getByText("chinese.png")).toBeVisible();
  expect(assetUploads).toBe(0);
  await imageSettings.getByRole("button", { name: "上传并应用" }).click();
  const img = page.locator(".mt-block-editor-image img");
  await expect(img).toBeVisible();
  await expect.poll(() => img.evaluate((el: HTMLImageElement) => el.naturalWidth)).toBeGreaterThan(0);
  expect(assetUploads).toBe(1);

  await page.getByRole("button", { name: "保存并记录历史" }).click();
  await expect.poll(() => patches.length).toBeGreaterThan(0);
  const savedPayload = JSON.stringify(patches.at(-1));
  expect(savedPayload).toContain("真实浏览器标题");
  expect(savedPayload).toContain(`"asset_id":"${uploadedAssetId}"`);
  expect(savedPayload).not.toContain("/api/v1/assets/");

  // Unsupported block types show a warning banner without corrupting saved drafts.
  const patchCountBefore = patches.length;
  await editor.click();
  await page.keyboard.press("End");
  await page.keyboard.press("Enter");
  await page.keyboard.type("/");
  await expect(slashList).toBeVisible();
  await slashList.getByRole("option", { name: "分割线" }).click();
  await expect(page.getByRole("region", { name: "笔记编辑器" }).getByRole("status")).toContainText("未应用此次编辑");
  expect(patches).toHaveLength(patchCountBefore);
});

test("笔记表格可自动保存、刷新重开并在历史中预览", async ({ page }) => {
  test.skip(baseline, "改版后的编辑器回归");
  let saved: Omit<typeof note, "document"> & { document: components["schemas"]["NoteDocument"] } = { ...note, document: { blocks: [
    { id: id(100), type: "paragraph" },
    { id: id(101), type: "paragraph", content: [{ type: "text", text: "粘贴位置" }] },
  ] } };
  let snapshot: (typeof saved & { snapshot_id: string; note_id: string; reason: string }) | undefined;
  await fixtures(page);
  await page.route(`**/api/v1/notes/${note.id}`, (route) => {
    if (route.request().method() === "PATCH") {
      const patch = route.request().postDataJSON();
      expect(patch.expected_version).toBe(saved.content_version);
      saved = { ...saved, title: patch.title, document: patch.document, content_version: saved.content_version + 1 };
    }
    return route.fulfill({ json: saved });
  });
  await page.route(url => url.pathname === `/api/v1/notes/${note.id}/snapshots`, (route) => {
    if (route.request().method() === "POST") {
      snapshot = { ...saved, snapshot_id: id(55), note_id: note.id, reason: "flush" };
      return route.fulfill({ status: 201, json: snapshot });
    }
    return route.fulfill({ json: list(snapshot ? [snapshot] : []) });
  });
  await page.goto(`http://127.0.0.1:4178/notes/${note.id}`);
  const editor = page.getByRole("textbox", { name: "笔记编辑器" });
  await editor.locator("p").first().click();
  await page.keyboard.type("/");
  await page.getByRole("listbox", { name: "插入块" }).getByRole("option", { name: "表格" }).click();
  const inserted = editor.locator("table").first();
  await inserted.locator("td").first().click();
  await page.keyboard.insertText("插入的项目");
  const table = (cell: string) => expect.objectContaining({
    type: "table",
    table: expect.objectContaining({
      rows: expect.arrayContaining([expect.objectContaining({
        cells: expect.arrayContaining([[expect.objectContaining({ type: "text", text: cell })]]),
      })]),
    }),
  });
  await expect.poll(() => saved.document.blocks).toContainEqual(table("插入的项目"));
  await inserted.locator("td").last().click();
  await page.getByRole("group", { name: "表格操作" }).getByRole("button", { name: "添加行", exact: true }).click();
  await expect(inserted.locator("tr")).toHaveCount(3);

  await editor.getByText("粘贴位置", { exact: true }).click();
  await page.keyboard.press("End");
  await editor.evaluate(element => {
    const clipboardData = new DataTransfer();
    clipboardData.setData("text/html", "<table><tr><th>项目</th><th>负责人</th></tr><tr><td><strong>粘贴的项目</strong></td><td></td></tr></table>");
    element.dispatchEvent(new ClipboardEvent("paste", { clipboardData, bubbles: true, cancelable: true }));
  });
  await expect(editor.locator("table")).toHaveCount(2);
  await page.getByRole("button", { name: "保存并记录历史" }).click();
  await expect.poll(() => snapshot?.document.blocks.find(block => block.id === id(101))?.children).toContainEqual(expect.objectContaining({
    type: "table",
    table: expect.objectContaining({
      header_rows: 1,
      rows: [
        { cells: [[{ type: "text", text: "项目" }], [{ type: "text", text: "负责人" }]] },
        { cells: [[{ type: "text", text: "粘贴的项目", marks: ["bold"] }], []] },
      ],
    }),
  }));
  await expect(page.getByText(/未应用此次编辑/)).toHaveCount(0);

  await page.reload();
  await expect(editor.locator("table")).toHaveCount(2);
  await expect(inserted.locator("tr")).toHaveCount(3);
  await expect(editor.locator("table").last().locator("th")).toHaveText(["项目", "负责人"]);
  await expect(editor.locator("table").last().locator("td")).toHaveText(["粘贴的项目", ""]);
  await page.getByRole("button", { name: "历史版本", exact: true }).click();
  await page.getByRole("button", { name: /预览 调研手记/ }).click();
  const preview = page.getByRole("region", { name: "历史预览" });
  await expect(preview.locator("table")).toHaveCount(2);
  await expect(preview.locator("table").last()).toContainText("粘贴的项目");
  await expect(preview.locator("[contenteditable=true]")).toHaveCount(0);
});

test("笔记编辑页标题输入框在常态、悬停与聚焦态均隐藏边框", async ({ page }) => {
  test.skip(baseline, "改版后的样式回归");
  await fixtures(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(`http://127.0.0.1:4178/notes/${note.id}`);

  const titleInput = page.getByRole("textbox", { name: "笔记标题" });
  await expect(titleInput).toHaveValue(note.title);

  const expectBorderless = async () => {
    const styles = await titleInput.evaluate(el => {
      const read = (node: Element) => {
        const s = getComputedStyle(node);
        return {
          borderWidths: [s.borderTopWidth, s.borderRightWidth, s.borderBottomWidth, s.borderLeftWidth],
          borderStyles: [s.borderTopStyle, s.borderRightStyle, s.borderBottomStyle, s.borderLeftStyle],
          boxShadow: s.boxShadow,
          outlineStyle: s.outlineStyle,
        };
      };
      return {
        input: read(el),
        wrapper: el.parentElement ? read(el.parentElement) : null,
      };
    });

    expect(styles.input.borderWidths).toEqual(["0px", "0px", "0px", "0px"]);
    expect(styles.input.borderStyles).toEqual(["none", "none", "none", "none"]);
    expect(styles.input.boxShadow).toBe("none");
    expect(styles.input.outlineStyle).toBe("none");
    expect(styles.wrapper?.borderWidths).toEqual(["0px", "0px", "0px", "0px"]);
    expect(styles.wrapper?.boxShadow).toBe("none");
    expect(styles.wrapper?.outlineStyle).toBe("none");
  };

  await expectBorderless();

  await titleInput.hover();
  await expectBorderless();

  await titleInput.click();
  await expect(titleInput).toBeFocused();
  await expectBorderless();

  await page.keyboard.press("Tab");
  await page.keyboard.press("Shift+Tab");
  await expect(titleInput).toBeFocused();
  await expectBorderless();

  await titleInput.fill("无边框标题验证");
  await expect(titleInput).toHaveValue("无边框标题验证");
});

test("笔记侧栏原生拖放时目录保持稳定，支持嵌套、失败回退和刷新", async ({ page }) => {
  test.skip(baseline, "改版后的笔记侧栏 Tree 回归");
  await fixtures(page);
  const treeNotes = [
    note,
    { ...note, id: id(16), title: "方案备忘", position: 1 },
    { ...note, id: id(17), title: "会议记录", position: 2 },
    { ...note, id: id(19), title: "访谈大纲", position: 3 },
    { ...note, id: id(15), title: "后续问题", parent_id: note.id, position: 0 },
    { ...note, id: id(18), title: "调研附件", parent_id: note.id, position: 1 },
  ].map(item => ({ ...item }));
  const moves: unknown[] = [];
  let failMove = false;
  let unblockFailedMove: (() => void) | undefined;
  let pauseRefresh = false;
  let unblockRefresh: (() => void) | undefined;
  await page.route("**/api/v1/notes**", async route => {
    const request = route.request();
    const pathname = new URL(request.url()).pathname;
    if (pathname === "/api/v1/notes") {
      if (pauseRefresh) {
        pauseRefresh = false;
        await new Promise<void>(resolve => { unblockRefresh = resolve; });
      }
      return route.fulfill({ json: list(treeNotes) });
    }
    const moved = treeNotes.find(item => pathname === `/api/v1/notes/${item.id}/move`);
    if (request.method() === "GET") {
      const detail = treeNotes.find(item => pathname === `/api/v1/notes/${item.id}`);
      if (detail) return route.fulfill({ json: detail });
    }
    if (request.method() === "POST" && pathname === `/api/v1/notes/${note.id}/snapshots`) {
      return route.fulfill({ json: { ...treeNotes[0], note_id: note.id, snapshot_id: id(51), reason: "flush" } });
    }
    if (!moved || request.method() !== "POST") return route.fallback();

    const body = request.postDataJSON();
    moves.push({ id: moved.id, ...body });
    if (failMove) {
      failMove = false;
      await new Promise<void>(resolve => { unblockFailedMove = resolve; });
      return route.fulfill({ status: 503, json: { code: "dependency_unavailable", message: "排序暂时失败" } });
    }
    const previousParent = moved.parent_id;
    moved.parent_id = body.parent_id;
    const siblings = treeNotes.filter(item => item.parent_id === moved.parent_id && item !== moved)
      .sort((a, b) => a.position - b.position);
    siblings.splice(body.position, 0, moved);
    siblings.forEach((item, index) => { item.position = index; });
    if (previousParent !== moved.parent_id) {
      treeNotes.filter(item => item.parent_id === previousParent)
        .sort((a, b) => a.position - b.position)
        .forEach((item, index) => { item.position = index; });
    }
    moved.content_version++;
    return route.fulfill({ json: moved });
  });

  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("http://127.0.0.1:4178/notes");
  const sidebar = page.getByRole("complementary", { name: "笔记目录" });
  const row = (name: string) => sidebar.getByRole("treeitem", { name, exact: true });
  const expectOrder = (level: number, titles: string[]) => expect(sidebar.locator(`[role="treeitem"][aria-level="${level}"] span[title]`)).toHaveText(titles);
  const drag = async (from: string, to: string, ratio: number) => {
    const target = row(to);
    const box = await target.boundingBox();
    await row(from).dragTo(target, { sourcePosition: { x: 80, y: 16 }, targetPosition: { x: 80, y: box!.height * ratio } });
  };
  await expect(row("会议记录")).toHaveAttribute("draggable", "true");
  await row("调研手记").press("ArrowRight");
  await expect(row("后续问题")).toBeVisible();
  await drag("调研手记", "后续问题", 0.5);
  await drag("会议记录", "方案备忘", 0.9);

  expect(moves).toEqual([]);
  const tree = sidebar.getByRole("tree", { name: "笔记树" });
  const top = (await tree.boundingBox())!.y;
  pauseRefresh = true;
  try {
    await drag("会议记录", "方案备忘", 0.1);
    await expect.poll(() => Boolean(unblockRefresh)).toBe(true);
    await expect(row("会议记录")).toHaveAttribute("draggable", "false");
    expect((await tree.boundingBox())!.y).toBe(top);
    await expectOrder(1, ["调研手记", "会议记录", "方案备忘", "访谈大纲"]);
  } finally {
    unblockRefresh?.();
  }
  await expect(row("会议记录")).toHaveAttribute("draggable", "true");

  failMove = true;
  try {
    await drag("会议记录", "访谈大纲", 0.9);
    await expect.poll(() => Boolean(unblockFailedMove)).toBe(true);
    await expectOrder(1, ["调研手记", "方案备忘", "访谈大纲", "会议记录"]);
  } finally {
    unblockFailedMove?.();
  }
  await expect(page.getByText("排序暂时失败")).toBeVisible();
  await expectOrder(1, ["调研手记", "会议记录", "方案备忘", "访谈大纲"]);
  await expect(row("会议记录")).toHaveAttribute("draggable", "true");

  await drag("会议记录", "访谈大纲", 0.9);
  await expectOrder(1, ["调研手记", "方案备忘", "访谈大纲", "会议记录"]);
  await expect(row("后续问题")).toHaveAttribute("draggable", "true");
  await drag("后续问题", "调研附件", 0.9);
  await expectOrder(2, ["调研附件", "后续问题"]);
  await expect(row("会议记录")).toHaveAttribute("draggable", "true");

  await drag("会议记录", "方案备忘", 0.5);
  await expect(row("方案备忘")).toHaveAttribute("aria-expanded", "true");
  await expect(row("会议记录")).toHaveAttribute("aria-level", "2");
  await expect(row("会议记录")).toHaveAttribute("draggable", "true");
  await expect(row("方案备忘").locator(".mt-tree-icon .lucide-file")).toBeVisible();
  await expect(row("会议记录").locator(".mt-tree-icon .lucide-file")).toBeVisible();

  await drag("会议记录", "调研附件", 0.1);
  await expectOrder(2, ["会议记录", "调研附件", "后续问题"]);
  await expect(row("方案备忘")).not.toHaveAttribute("aria-expanded", /.+/);
  await expect(row("会议记录")).toHaveAttribute("draggable", "true");

  await drag("调研手记", "方案备忘", 0.5);
  await expectOrder(1, ["方案备忘", "访谈大纲"]);
  await expect(row("调研手记")).toHaveAttribute("aria-level", "2");
  await expect(row("调研附件")).toHaveAttribute("aria-level", "3");
  await expect(row("调研手记")).toHaveAttribute("draggable", "true");

  await drag("调研手记", "方案备忘", 0.1);
  await expectOrder(1, ["调研手记", "方案备忘", "访谈大纲"]);
  await expectOrder(2, ["会议记录", "调研附件", "后续问题"]);
  await expect(row("调研手记")).toHaveAttribute("draggable", "true");
  expect(moves).toEqual([
    { id: id(17), expected_version: 2, parent_id: null, position: 1 },
    { id: id(17), expected_version: 3, parent_id: null, position: 3 },
    { id: id(17), expected_version: 3, parent_id: null, position: 3 },
    { id: id(15), expected_version: 2, parent_id: note.id, position: 1 },
    { id: id(17), expected_version: 4, parent_id: id(16), position: 0 },
    { id: id(17), expected_version: 5, parent_id: note.id, position: 0 },
    { id: note.id, expected_version: 2, parent_id: id(16), position: 0 },
    { id: note.id, expected_version: 3, parent_id: null, position: 0 },
  ]);

  await page.reload();
  await expectOrder(1, ["调研手记", "方案备忘", "访谈大纲"]);
  await row("调研手记").press("ArrowRight");
  await expectOrder(2, ["会议记录", "调研附件", "后续问题"]);
});

test("笔记左侧 Tree 节点右侧按钮仅在悬停时显示，支持添加子页面与悬停弹出常用操作菜单", async ({ page }) => {
  test.skip(baseline, "改版后的笔记侧栏 Tree 回归");
  await fixtures(page);
  const secondNote = {
    ...note,
    id: id(16),
    title: "方案备忘",
    position: 1,
    content_version: 1,
    document: { blocks: [{ id: id(160), type: "paragraph", content: [{ type: "text", text: "方案备忘正文", styles: {} }] }] },
  };
  let treeNotes = [
    { ...note },
    { ...note, id: id(15), title: "后续问题", parent_id: note.id, position: 0 },
    secondNote,
  ];
  let createdSeq = 200;

  await page.addInitScript(() => {
    const copied: string[] = [];
    (window as unknown as { __copiedLinks?: string[] }).__copiedLinks = copied;
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        writeText: async (text: string) => { copied.push(text); },
      },
    });
  });

  await page.route("**/api/v1/notes**", async route => {
    const req = route.request();
    const url = new URL(req.url());
    const rel = url.pathname.replace("/api/v1", "");
    if (rel === "/notes" && req.method() === "GET") {
      return route.fulfill({ json: list(treeNotes) });
    }
    if (rel === "/notes" && req.method() === "POST") {
      const body = req.postDataJSON();
      const created = {
        ...note,
        id: id(++createdSeq),
        title: body.title,
        parent_id: body.parent_id ?? null,
        document: body.document,
        position: treeNotes.filter(item => item.parent_id === (body.parent_id ?? null)).length,
        content_version: 1,
      };
      treeNotes = [...treeNotes, created];
      return route.fulfill({ status: 201, json: created });
    }
    const match = rel.match(/^\/notes\/([0-9a-f-]+)(?:\/(move|snapshots))?$/i);
    if (match) {
      const targetId = match[1]!;
      const sub = match[2];
      const existing = treeNotes.find(item => item.id === targetId);
      if (sub === "snapshots") {
        if (req.method() === "POST" && existing) {
          return route.fulfill({ status: 201, json: { ...existing, note_id: existing.id, snapshot_id: id(51), reason: "flush" } });
        }
        return route.fulfill({ json: list([]) });
      }
      if (!existing) return route.fulfill({ status: 404, json: { code: "not_found", message: "not found" } });
      if (req.method() === "GET") return route.fulfill({ json: existing });
      if (req.method() === "PATCH") {
        const body = req.postDataJSON();
        const updated = { ...existing, ...body, content_version: existing.content_version + 1 };
        treeNotes = treeNotes.map(item => item.id === targetId ? updated : item);
        return route.fulfill({ json: updated });
      }
      if (sub === "move" && req.method() === "POST") {
        const body = req.postDataJSON();
        const updated = { ...existing, parent_id: body.parent_id, position: body.position, content_version: existing.content_version + 1 };
        treeNotes = treeNotes.map(item => item.id === targetId ? updated : item);
        return route.fulfill({ json: updated });
      }
      if (req.method() === "DELETE") {
        treeNotes = treeNotes.filter(item => item.id !== targetId);
        return route.fulfill({ status: 204 });
      }
    }
    return route.fallback();
  });
  await page.route(/\/api\/v1\/entities\/note\/[^/]+\/tags/, route => route.fulfill({ json: list([]) }));

  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("http://127.0.0.1:4178/notes");

  const sidebar = page.getByRole("complementary", { name: "笔记目录" });
  const secondRow = sidebar.getByRole("treeitem", { name: "方案备忘" });
  await expect(secondRow).toBeVisible();

  // Move pointer away from sidebar so tree actions are hidden at rest.
  const titleInput = page.getByRole("textbox", { name: "笔记标题" });
  await titleInput.click();
  await titleInput.hover();

  const readActionStyles = (row: Locator) => row.locator('.mt-tree-action[data-visibility="hover"]').evaluateAll(nodes =>
    nodes.map(el => {
      const style = getComputedStyle(el);
      return { opacity: style.opacity, pointerEvents: style.pointerEvents };
    }),
  );

  expect(await readActionStyles(secondRow)).toEqual([
    { opacity: "0", pointerEvents: "none" },
    { opacity: "0", pointerEvents: "none" },
  ]);

  // Hovering the row reveals both the "+" add-subpage button and the "..." menu trigger.
  await secondRow.hover();
  await expect.poll(() => readActionStyles(secondRow)).toEqual([
    { opacity: "1", pointerEvents: "auto" },
    { opacity: "1", pointerEvents: "auto" },
  ]);

  const addSubpageBtn = secondRow.getByRole("button", { name: "新建子页面：方案备忘" });
  const moreBtn = secondRow.getByRole("button", { name: "更多页面操作：方案备忘" });
  await expect(addSubpageBtn).toBeVisible();
  await expect(moreBtn).toBeVisible();

  // Hovering the "..." button pops open the action menu without needing a click.
  await moreBtn.hover();
  const menu = page.getByRole("menu", { name: "页面操作：方案备忘" });
  await expect(menu).toBeVisible();
  for (const label of ["新建子页面", "重命名", "创建副本", "移动到…", "复制链接", "删除页面"]) {
    await expect(menu.getByRole("menuitem", { name: label })).toBeVisible();
  }

  // Copy link via the hover-opened menu.
  await menu.getByRole("menuitem", { name: "复制链接" }).click();
  await expect(page.getByText("已复制页面链接")).toBeVisible();
  expect(await page.evaluate(() => (window as unknown as { __copiedLinks?: string[] }).__copiedLinks)).toEqual([
    `http://127.0.0.1:4178/notes/${secondNote.id}`,
  ]);

  // Rename via the hover-opened menu.
  await secondRow.hover();
  await moreBtn.hover();
  await expect(menu).toBeVisible();
  await menu.getByRole("menuitem", { name: "重命名" }).click();
  const renameDialog = page.getByRole("dialog", { name: "重命名页面" });
  await expect(renameDialog).toBeVisible();
  await renameDialog.getByRole("textbox", { name: "页面标题" }).fill("方案备忘 v2");
  await renameDialog.getByRole("button", { name: "确认重命名" }).click();
  await expect(renameDialog).toBeHidden();
  const renamedRow = sidebar.getByRole("treeitem", { name: "方案备忘 v2", exact: true });
  await expect(renamedRow).toBeVisible();

  // Duplicate via the hover-opened menu.
  await renamedRow.hover();
  await renamedRow.getByRole("button", { name: "更多页面操作：方案备忘 v2" }).hover();
  const renamedMenu = page.getByRole("menu", { name: "页面操作：方案备忘 v2" });
  await expect(renamedMenu).toBeVisible();
  await renamedMenu.getByRole("menuitem", { name: "创建副本" }).click();
  await expect(page.getByRole("textbox", { name: "笔记标题" })).toHaveValue("方案备忘 v2（副本）");
  const copyRow = sidebar.getByRole("treeitem", { name: "方案备忘 v2（副本）", exact: true });
  await expect(copyRow).toBeVisible();

  // Add a subpage directly via the hover "+" button.
  await renamedRow.hover();
  await renamedRow.getByRole("button", { name: "新建子页面：方案备忘 v2" }).click();
  await expect(page.getByRole("textbox", { name: "笔记标题" })).toHaveValue("未命名笔记");
  await expect(renamedRow).toHaveAttribute("aria-expanded", "true");
  await expect(sidebar.getByRole("treeitem", { name: "未命名笔记", exact: true })).toBeVisible();

  // Delete an inactive page via the hover-opened menu and confirm undo receipt appears.
  await copyRow.hover();
  await copyRow.getByRole("button", { name: "更多页面操作：方案备忘 v2（副本）" }).hover();
  const copyMenu = page.getByRole("menu", { name: "页面操作：方案备忘 v2（副本）" });
  await expect(copyMenu).toBeVisible();
  await copyMenu.getByRole("menuitem", { name: "删除页面" }).click();
  const confirmDialog = page.getByRole("alertdialog", { name: "删除笔记" });
  await expect(confirmDialog).toBeVisible();
  await confirmDialog.getByRole("button", { name: "确认删除" }).click();
  await expect(copyRow).toHaveCount(0);
  await expect(page.getByRole("button", { name: "恢复本次删除 方案备忘 v2（副本）" })).toBeVisible();

  expect(await page.getByRole("main").evaluate(main => main.scrollWidth - main.clientWidth)).toBeLessThanOrEqual(1);
});
