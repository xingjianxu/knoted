import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { StrictMode } from "react";
import { describe, expect, it } from "vitest";

import { createCursorState, cursorReducer, type CursorState } from "../../src/lib/cursor-state";
import { UiProvider } from "../../src/lib/ui-provider";
import { T19TreeList } from "../fixtures/t19-tree-list";

// These assertions catch lost history, cursor decoding, accidental resets and
// mutation of state held by a previous render. Expected stacks are hand-derived.
describe("t19 application cursor state", () => {
  it("starts on the first page with an optional filter", () => {
    expect(createCursorState()).toEqual({ filterKey: "", cursor: null, previous: [] });
    expect(createCursorState("tag:甲")).toEqual({ filterKey: "tag:甲", cursor: null, previous: [] });
  });

  it("t19_4 traverses three pages and pops back to the first page", () => {
    const first = createCursorState("all");
    const second = cursorReducer(first, { type: "next", nextCursor: "opaque:二+/=" });
    expect(second).toEqual({ filterKey: "all", cursor: "opaque:二+/=", previous: [null] });
    const third = cursorReducer(second, { type: "next", nextCursor: "v3.%2F?甲" });
    expect(third).toEqual({ filterKey: "all", cursor: "v3.%2F?甲", previous: [null, "opaque:二+/="] });
    const back = cursorReducer(third, { type: "previous" });
    expect(back).toEqual(second);
    expect(cursorReducer(back, { type: "previous" })).toEqual(first);
  });

  it("does not navigate past boundaries and preserves state identity for no-ops", () => {
    const first = createCursorState();
    expect(cursorReducer(first, { type: "previous" })).toBe(first);
    const second = cursorReducer(first, { type: "next", nextCursor: "last" });
    expect(cursorReducer(second, { type: "next", nextCursor: null })).toBe(second);
  });

  it("treats even an empty non-null cursor as opaque rather than falsy", () => {
    expect(cursorReducer(createCursorState(), { type: "next", nextCursor: "" }))
      .toEqual({ filterKey: "", cursor: "", previous: [null] });
  });

  it("resets only on a different filter, retaining history for the same filter", () => {
    const state = cursorReducer(createCursorState("all"), { type: "next", nextCursor: "page2" });
    expect(cursorReducer(state, { type: "filter", filterKey: "all" })).toBe(state);
    expect(cursorReducer(state, { type: "filter", filterKey: "selected" }))
      .toEqual({ filterKey: "selected", cursor: null, previous: [] });
  });

  it("does not mutate frozen inputs or previously returned states", () => {
    const state: CursorState = Object.freeze({
      filterKey: "all", cursor: "second", previous: Object.freeze([null]),
    });
    expect(cursorReducer(state, { type: "next", nextCursor: "third" }))
      .toEqual({ filterKey: "all", cursor: "third", previous: [null, "second"] });
    expect(cursorReducer(state, { type: "previous" }))
      .toEqual({ filterKey: "all", cursor: null, previous: [] });
    expect(cursorReducer(state, { type: "filter", filterKey: "new" }))
      .toEqual({ filterKey: "new", cursor: null, previous: [] });
    expect(state).toEqual({ filterKey: "all", cursor: "second", previous: [null] });
    const a = createCursorState();
    const b = createCursorState();
    expect(a.previous).not.toBe(b.previous);
  });
});

function renderTreeList() {
  return render(
    <StrictMode>
      <UiProvider>
        <T19TreeList />
      </UiProvider>
    </StrictMode>,
  );
}

function treeToggle(tree: HTMLElement, name: string): HTMLElement {
  const item = within(tree).getByRole("treeitem", { name });
  const toggle = item.querySelector<HTMLElement>(".mt-tree-toggle");
  if (!toggle) throw new Error(`tree item ${name} has no toggle`);
  return toggle;
}

describe("t19 mtone tree/list/confirm consumption", () => {
  it("t19_1 mouse toggle expands without selecting and controlled state stays in sync", () => {
    renderTreeList();
    const tree = screen.getByRole("tree", { name: "笔记树" });
    const project = within(tree).getByRole("treeitem", { name: "项目" });
    expect(project).toHaveAttribute("aria-expanded", "false");
    expect(screen.getByLabelText("树选择")).toHaveTextContent("无");
    fireEvent.click(treeToggle(tree, "项目"));
    expect(project).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByLabelText("树展开")).toHaveTextContent("project");
    expect(screen.getByLabelText("树选择")).toHaveTextContent("无");
    expect(within(tree).getByRole("treeitem", { name: "项目甲" })).toBeVisible();
  });

  it("t19_1 keyboard expands without selecting, then collapses and selects on Enter/Space", () => {
    renderTreeList();
    const tree = screen.getByRole("tree", { name: "笔记树" });
    const project = within(tree).getByRole("treeitem", { name: "项目" });
    fireEvent.keyDown(project, { key: "ArrowRight" });
    expect(screen.getByLabelText("树展开")).toHaveTextContent("project");
    expect(screen.getByLabelText("树选择")).toHaveTextContent("无");
    fireEvent.keyDown(project, { key: "Enter" });
    expect(screen.getByLabelText("树选择")).toHaveTextContent("project");
    fireEvent.keyDown(project, { key: "ArrowLeft" });
    expect(screen.getByLabelText("树展开")).toHaveTextContent("无");
    const personal = within(tree).getByRole("treeitem", { name: "个人" });
    fireEvent.keyDown(personal, { key: " " });
    expect(screen.getByLabelText("树选择")).toHaveTextContent("personal");
  });

  it("t19_1 disabled tree nodes never select or expand, and stay skipped in keyboard order", () => {
    renderTreeList();
    const tree = screen.getByRole("tree", { name: "笔记树" });
    const locked = within(tree).getByRole("treeitem", { name: "已锁定" });
    expect(locked).toHaveAttribute("aria-disabled", "true");
    expect(locked).toHaveAttribute("tabindex", "-1");
    fireEvent.click(locked);
    expect(screen.getByLabelText("树选择")).toHaveTextContent("无");
    fireEvent.click(treeToggle(tree, "已锁定"));
    expect(screen.getByLabelText("树展开")).toHaveTextContent("无");
    expect(within(tree).queryByRole("treeitem", { name: "锁定的子项" })).not.toBeInTheDocument();
    fireEvent.click(treeToggle(tree, "项目"));
    const projectB = within(tree).getByRole("treeitem", { name: "项目乙" });
    expect(projectB).toHaveAttribute("aria-disabled", "true");
    fireEvent.click(projectB);
    expect(screen.getByLabelText("树选择")).toHaveTextContent("无");
  });

  it("t19_2 empty/error/loading states are mutually exclusive and recover", () => {
    renderTreeList();
    expect(screen.getByText("首页笔记一")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "模拟加载" }));
    expect(screen.getByRole("status", { name: "加载笔记" })).toBeVisible();
    expect(screen.queryByRole("list", { name: "笔记列表" })).not.toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(screen.queryByText("没有笔记")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "模拟失败" }));
    expect(screen.getByRole("alert")).toHaveTextContent("加载失败");
    expect(screen.queryByRole("status", { name: "加载笔记" })).not.toBeInTheDocument();
    expect(screen.queryByRole("list", { name: "笔记列表" })).not.toBeInTheDocument();
    expect(screen.queryByText("没有笔记")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "模拟空列表" }));
    expect(screen.getByText("没有笔记")).toBeVisible();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(screen.queryByRole("status", { name: "加载笔记" })).not.toBeInTheDocument();
    expect(screen.queryByRole("list", { name: "笔记列表" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "恢复数据" }));
    expect(screen.getByText("首页笔记一")).toBeVisible();
  });

  it("t19_3 object copy, cancel zero callbacks, pending single submit and visible failure", async () => {
    renderTreeList();
    const trigger = screen.getByRole("button", { name: "归档项目甲" });
    fireEvent.click(trigger);
    const dialog = await screen.findByRole("alertdialog", { name: "确认归档“项目甲”" });
    expect(dialog).toHaveAccessibleDescription(/项目甲/);
    fireEvent.click(within(dialog).getByRole("button", { name: "取消" }));
    await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument());
    expect(screen.getByLabelText("确认结果")).toHaveTextContent("请求：0；成功：0");
    fireEvent.click(trigger);
    const reopened = await screen.findByRole("alertdialog", { name: "确认归档“项目甲”" });
    fireEvent.click(within(reopened).getByRole("button", { name: "确认归档" }));
    const pending = within(reopened).getByRole("button", { name: "…" });
    expect(pending).toBeDisabled();
    expect(within(reopened).getByRole("button", { name: "取消" })).toBeDisabled();
    fireEvent.click(pending);
    expect(screen.getByLabelText("确认结果")).toHaveTextContent("请求：1；成功：0");
    fireEvent.click(within(reopened).getByRole("button", { name: "模拟请求失败" }));
    expect(await within(reopened).findByRole("alert")).toHaveTextContent("操作失败，请重试。");
    expect(reopened).toBeVisible();
    expect(screen.getByLabelText("确认结果")).toHaveTextContent("请求：1；成功：0");
    fireEvent.click(within(reopened).getByRole("button", { name: "确认归档" }));
    expect(within(reopened).queryByRole("alert")).not.toBeInTheDocument();
    fireEvent.click(within(reopened).getByRole("button", { name: "模拟请求成功" }));
    await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument());
    expect(screen.getByLabelText("确认结果")).toHaveTextContent("请求：2；成功：1");
  });

  it("t19_4 pages without total and filter change clears the cursor stack", () => {
    renderTreeList();
    const previous = screen.getByRole("button", { name: "上一页" });
    const next = screen.getByRole("button", { name: "下一页" });
    expect(previous).toBeDisabled();
    expect(next).toBeEnabled();
    expect(screen.getByText("首页笔记一")).toBeVisible();
    expect(screen.getByLabelText("列表状态")).toHaveTextContent("游标：首屏；历史：0；筛选：all");
    fireEvent.click(next);
    expect(screen.getByText("第二页笔记三")).toBeVisible();
    expect(previous).toBeEnabled();
    expect(screen.getByLabelText("列表状态")).toHaveTextContent("游标：opaque:二+/=；历史：1；筛选：all");
    fireEvent.click(next);
    expect(screen.getByText("末页笔记五")).toBeVisible();
    expect(next).toBeDisabled();
    expect(screen.getByLabelText("列表状态")).toHaveTextContent("游标：v3.%2F?甲；历史：2；筛选：all");
    fireEvent.click(previous);
    expect(screen.getByText("第二页笔记三")).toBeVisible();
    fireEvent.click(previous);
    expect(screen.getByText("首页笔记一")).toBeVisible();
    expect(previous).toBeDisabled();
    fireEvent.click(next);
    fireEvent.click(next);
    fireEvent.click(screen.getByRole("button", { name: "筛选星标" }));
    expect(screen.getByLabelText("列表状态")).toHaveTextContent("游标：首屏；历史：0；筛选：starred");
    expect(previous).toBeDisabled();
    expect(screen.getByText("首页笔记一")).toBeVisible();
  });
});
