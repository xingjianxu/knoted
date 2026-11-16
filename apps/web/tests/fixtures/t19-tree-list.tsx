import {
  Alert,
  Button,
  ConfirmDialog,
  Empty,
  List,
  ListItem,
  Spinner,
  Tree,
  type TreeItem,
} from "@bixlab/mtone";
import { useReducer, useRef, useState } from "react";

import { createCursorState, cursorReducer } from "../../src/lib/cursor-state";

type StubNote = { id: string; title: string };
type Page = { items: StubNote[]; next_cursor: string | null };

const treeItems: TreeItem[] = [
  {
    id: "project",
    label: "项目",
    children: [
      { id: "project-a", label: "项目甲" },
      { id: "project-b", label: "项目乙", disabled: true },
    ],
  },
  {
    id: "personal",
    label: "个人",
    children: [{ id: "personal-a", label: "个人甲" }],
  },
  {
    id: "locked",
    label: "已锁定",
    disabled: true,
    children: [{ id: "locked-a", label: "锁定的子项" }],
  },
];

// Cursor-shaped stub pages mirror the items/next_cursor contract without any
// total/offset field. Cursors stay opaque: the fixture never decodes them.
const firstPage: Page = {
  items: [
    { id: "n-1", title: "首页笔记一" },
    { id: "n-2", title: "首页笔记二" },
  ],
  next_cursor: "opaque:二+/=",
};

const pages: Record<string, Page> = {
  first: firstPage,
  "opaque:二+/=": {
    items: [
      { id: "n-3", title: "第二页笔记三" },
      { id: "n-4", title: "第二页笔记四" },
    ],
    next_cursor: "v3.%2F?甲",
  },
  "v3.%2F?甲": {
    items: [{ id: "n-5", title: "末页笔记五" }],
    next_cursor: null,
  },
};

function pageFor(cursor: string | null): Page {
  if (cursor === null) return firstPage;
  return pages[cursor] ?? firstPage;
}

// Test-only composition. Production owns only the cursor reducer; every
// Tree/List/Empty/Spinner/Alert/ConfirmDialog usage stays in this fixture and
// business DTOs are mapped to public props instead of entering mtone.
export function T19TreeList() {
  const [treeValue, setTreeValue] = useState<string | undefined>();
  const [expanded, setExpanded] = useState<string[]>([]);
  const [listStatus, setListStatus] = useState<"loading" | "error" | "empty" | "ready">("ready");
  const [state, dispatch] = useReducer(cursorReducer, undefined, () => createCursorState("all"));
  const [attempts, setAttempts] = useState(0);
  const [successes, setSuccesses] = useState(0);
  const request = useRef<{ resolve: () => void; reject: (error: Error) => void } | null>(null);

  const page = pageFor(state.cursor);
  const confirmObject = "项目甲";

  async function confirm() {
    setAttempts((value) => value + 1);
    await new Promise<void>((resolve, reject) => {
      request.current = { resolve, reject };
    });
    request.current = null;
    setSuccesses((value) => value + 1);
  }

  function settle(succeed: boolean) {
    const pending = request.current;
    request.current = null;
    if (succeed) pending?.resolve();
    else pending?.reject(new Error("fixture request rejected"));
  }

  return (
    <main className="mx-auto grid max-w-2xl gap-6 p-6">
      <h1>T19 树、游标列表与反馈消费夹具</h1>

      <section aria-label="树消费" className="grid gap-2">
        <Tree
          label="笔记树"
          items={treeItems}
          value={treeValue}
          onValueChange={setTreeValue}
          expanded={expanded}
          onExpandedChange={setExpanded}
        />
        <output aria-label="树选择">{treeValue ?? "无"}</output>
        <output aria-label="树展开">{expanded.join("，") || "无"}</output>
      </section>

      <section aria-label="游标列表" className="grid gap-2">
        <div className="flex gap-2">
          <Button onClick={() => setListStatus("loading")}>模拟加载</Button>
          <Button onClick={() => setListStatus("error")}>模拟失败</Button>
          <Button onClick={() => setListStatus("empty")}>模拟空列表</Button>
          <Button onClick={() => setListStatus("ready")}>恢复数据</Button>
        </div>
        <div className="flex gap-2">
          <Button onClick={() => dispatch({ type: "filter", filterKey: "all" })}>筛选全部</Button>
          <Button onClick={() => dispatch({ type: "filter", filterKey: "starred" })}>筛选星标</Button>
        </div>
        {listStatus === "loading" && <Spinner label="加载笔记" />}
        {listStatus === "error" && (
          <Alert tone="error" title="加载失败">请重试加载笔记</Alert>
        )}
        {listStatus === "empty" && <Empty title="没有笔记" description="换个筛选条件试试" />}
        {listStatus === "ready" && (
          <>
            <List aria-label="笔记列表">
              {page.items.map((item) => (
                <ListItem key={item.id} description="笔记条目">
                  {item.title}
                </ListItem>
              ))}
            </List>
            <div className="flex gap-2">
              <Button
                onClick={() => dispatch({ type: "previous" })}
                disabled={state.previous.length === 0}
              >
                上一页
              </Button>
              <Button
                onClick={() => {
                  if (page.next_cursor !== null) {
                    dispatch({ type: "next", nextCursor: page.next_cursor });
                  }
                }}
                disabled={page.next_cursor === null}
              >
                下一页
              </Button>
            </div>
          </>
        )}
        <output aria-label="列表状态">
          游标：{state.cursor ?? "首屏"}；历史：{state.previous.length}；筛选：{state.filterKey}
        </output>
      </section>

      <section aria-label="确认消费" className="grid gap-2">
        <ConfirmDialog
          title={`确认归档“${confirmObject}”`}
          description={`归档后，笔记“${confirmObject}”将从当前视图移除。`}
          trigger={<Button>归档项目甲</Button>}
          onConfirm={confirm}
          confirmText="确认归档"
        >
          <Button onClick={() => settle(false)}>模拟请求失败</Button>
          <Button onClick={() => settle(true)}>模拟请求成功</Button>
        </ConfirmDialog>
        <output aria-label="确认结果">请求：{attempts}；成功：{successes}</output>
      </section>
    </main>
  );
}
