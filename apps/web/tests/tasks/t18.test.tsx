import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { StrictMode } from "react";
import { describe, expect, it } from "vitest";

import { UiProvider } from "../../src/lib/ui-provider";
import { T18Notifications, T18Overlays } from "../fixtures/t18-overlays";

function renderOverlays() {
  return render(
    <StrictMode>
      <UiProvider><T18Overlays /></UiProvider>
    </StrictMode>,
  );
}

describe("t18 application notification registration", () => {
  it("t18_5 real success/error notifications render and close once under StrictMode", async () => {
    render(
      <StrictMode>
        <UiProvider><T18Notifications /></UiProvider>
      </StrictMode>,
    );
    for (const [action, title, body] of [
      ["成功通知", "保存成功", "笔记已保存"],
      ["失败通知", "保存失败", "请重试保存"],
    ] as const) {
      fireEvent.click(screen.getByRole("button", { name: action }));
      expect(await screen.findByText(title)).toBeVisible();
      expect(screen.getAllByText(title)).toHaveLength(1);
      expect(screen.getByText(body)).toBeVisible();
      // Published Toast exposes its close control to AT when focused/expanded.
      fireEvent.focus(screen.getByRole("dialog", { name: title }));
      fireEvent.click(screen.getByRole("button", { name: "关闭" }));
      await waitFor(() => expect(screen.queryByText(title)).not.toBeInTheDocument());
    }
  });
});

describe("t18 published overlay consumption characteristics", () => {
  it("t18_1/t18_2 controlled Dialog opens and closes without state divergence", async () => {
    renderOverlays();
    fireEvent.click(screen.getByRole("button", { name: "编辑笔记" }));
    const dialog = await screen.findByRole("dialog", { name: "编辑笔记" });
    expect(screen.getByLabelText("受控状态")).toHaveTextContent("Dialog：true");
    expect(within(dialog).getByRole("textbox", { name: "笔记标题" })).toHaveValue("消费验收");
    fireEvent.click(within(dialog).getByRole("button", { name: "完成编辑" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(screen.getByLabelText("受控状态")).toHaveTextContent("Dialog：false");
  });

  it("t18_3 dropdown selection records one action, disabled selection records none", async () => {
    renderOverlays();
    fireEvent.click(screen.getByRole("button", { name: "笔记菜单" }));
    const disabled = await screen.findByRole("menuitem", { name: "删除笔记" });
    expect(disabled).toHaveAttribute("aria-disabled", "true");
    fireEvent.click(disabled);
    expect(screen.getByLabelText("菜单动作")).toHaveTextContent("下拉：0；上下文：0；最后：无");
    fireEvent.click(screen.getByRole("menuitem", { name: "归档笔记" }));
    expect(screen.getByLabelText("菜单动作")).toHaveTextContent("下拉：1；上下文：0；最后：archive");
    await waitFor(() => expect(screen.queryByRole("menu")).not.toBeInTheDocument());
    expect(screen.getByLabelText("受控状态")).toHaveTextContent("Dropdown：false");
  });

  it("t18_3 context selection records one action and disabled remains inert", async () => {
    renderOverlays();
    fireEvent.contextMenu(screen.getByRole("button", { name: "笔记上下文区域" }), { button: 2 });
    const disabled = await screen.findByRole("menuitem", { name: "删除笔记" });
    fireEvent.click(disabled);
    expect(screen.getByLabelText("菜单动作")).toHaveTextContent("下拉：0；上下文：0；最后：无");
    fireEvent.click(screen.getByRole("menuitem", { name: "打开笔记" }));
    expect(screen.getByLabelText("菜单动作")).toHaveTextContent("下拉：0；上下文：1；最后：open");
    await waitFor(() => expect(screen.queryByRole("menu")).not.toBeInTheDocument());
    expect(screen.getByLabelText("受控状态")).toHaveTextContent("Context：false");
  });

  it("t18_2 Popover uses the same controlled state for trigger and content dismissal", async () => {
    renderOverlays();
    fireEvent.click(screen.getByRole("button", { name: "查看信息" }));
    const dialog = await screen.findByRole("dialog", { name: "笔记信息" });
    expect(screen.getByLabelText("受控状态")).toHaveTextContent("Popover：true");
    fireEvent.click(within(dialog).getByRole("button", { name: "收起信息" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(screen.getByLabelText("受控状态")).toHaveTextContent("Popover：false");
  });

  it("t18_4 cancel does not submit; async rejection stays open and retry succeeds once", async () => {
    renderOverlays();
    const trigger = screen.getByRole("button", { name: "归档确认" });
    fireEvent.click(trigger);
    fireEvent.click(await screen.findByRole("button", { name: "取消" }));
    await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument());
    expect(screen.getByLabelText("确认结果")).toHaveTextContent("请求：0；成功：0");
    fireEvent.click(trigger);
    const dialog = await screen.findByRole("alertdialog", { name: "确认归档" });
    fireEvent.click(within(dialog).getByRole("button", { name: "确认归档" }));
    const pending = within(dialog).getByRole("button", { name: "…" });
    expect(pending).toBeDisabled();
    expect(within(dialog).getByRole("button", { name: "取消" })).toBeDisabled();
    fireEvent.click(pending);
    expect(screen.getByLabelText("确认结果")).toHaveTextContent("请求：1；成功：0");
    fireEvent.click(within(dialog).getByRole("button", { name: "模拟请求失败" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("操作失败，请重试。");
    expect(dialog).toBeVisible();
    expect(screen.getByLabelText("确认结果")).toHaveTextContent("请求：1；成功：0");
    fireEvent.click(within(dialog).getByRole("button", { name: "确认归档" }));
    expect(within(dialog).queryByRole("alert")).not.toBeInTheDocument();
    fireEvent.click(within(dialog).getByRole("button", { name: "模拟请求成功" }));
    await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument());
    expect(screen.getByLabelText("确认结果")).toHaveTextContent("请求：2；成功：1");
    expect(screen.getByLabelText("受控状态")).toHaveTextContent("Confirm：false");
  });
});
