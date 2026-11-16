import {
  Button,
  ConfirmDialog,
  ContextMenu,
  DropdownMenu,
  Input,
  Modal,
  Popover,
  useToast,
  type MenuItem,
} from "@bixlab/mtone";
import { useRef, useState } from "react";

const items: MenuItem[] = [
  { key: "open", label: "打开笔记" },
  { key: "delete", label: "删除笔记", disabled: true },
  { key: "archive", label: "归档笔记" },
];

// Test-only business compositions. No overlay, focus, or notification primitives
// are implemented here; production uses the same UiProvider as this fixture.
export function T18Overlays() {
  const [modalOpen, setModalOpen] = useState(false);
  const [hasTrigger, setHasTrigger] = useState(true);
  const [popoverOpen, setPopoverOpen] = useState(false);
  const [dropdownOpen, setDropdownOpen] = useState(false);
  const [contextOpen, setContextOpen] = useState(false);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [actions, setActions] = useState({ dropdown: 0, context: 0, last: "无" });
  const [attempts, setAttempts] = useState(0);
  const [successes, setSuccesses] = useState(0);
  const request = useRef<{ resolve: () => void; reject: (error: Error) => void } | null>(null);

  function select(source: "dropdown" | "context", key: string) {
    setActions((previous) => ({
      ...previous,
      [source]: previous[source] + 1,
      last: key,
    }));
  }

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
    <main className="grid max-w-2xl gap-4 p-6">
      <h1>T18 弹层消费夹具</h1>
      <Button>页面外部按钮</Button>
      <Modal
        title="编辑笔记"
        description="测试受控弹层与焦点生命周期"
        open={modalOpen}
        onOpenChange={setModalOpen}
        trigger={hasTrigger ? <Button>编辑笔记</Button> : undefined}
        footer={<Button onClick={() => setModalOpen(false)}>完成编辑</Button>}
      >
        <Input aria-label="笔记标题" defaultValue="消费验收" />
        <Button onClick={() => setHasTrigger(false)}>卸载触发点</Button>
      </Modal>
      <Popover
        title="笔记信息"
        trigger={<Button>查看信息</Button>}
        content={<Button onClick={() => setPopoverOpen(false)}>收起信息</Button>}
        open={popoverOpen}
        onOpenChange={setPopoverOpen}
      />
      <DropdownMenu
        trigger={<Button>笔记菜单</Button>}
        items={items}
        onSelect={(key) => select("dropdown", key)}
        open={dropdownOpen}
        onOpenChange={setDropdownOpen}
      />
      <ContextMenu
        items={items}
        onSelect={(key) => select("context", key)}
        open={contextOpen}
        onOpenChange={setContextOpen}
      >
        <Button>笔记上下文区域</Button>
      </ContextMenu>
      <output aria-label="菜单动作">
        下拉：{actions.dropdown}；上下文：{actions.context}；最后：{actions.last}
      </output>
      <ConfirmDialog
        title="确认归档"
        description="取消不提交；请求失败后可重试"
        trigger={<Button>归档确认</Button>}
        open={confirmOpen}
        onOpenChange={setConfirmOpen}
        onConfirm={confirm}
        confirmText="确认归档"
      >
        <Button onClick={() => settle(false)}>模拟请求失败</Button>
        <Button onClick={() => settle(true)}>模拟请求成功</Button>
      </ConfirmDialog>
      <output aria-label="确认结果">请求：{attempts}；成功：{successes}</output>
      <output aria-label="受控状态">
        Modal：{String(modalOpen)}；Popover：{String(popoverOpen)}；
        Dropdown：{String(dropdownOpen)}；Context：{String(contextOpen)}；
        Confirm：{String(confirmOpen)}
      </output>
      <T18Notifications />
    </main>
  );
}

export function T18Notifications() {
  const toast = useToast();
  return (
    <section aria-label="应用通知" className="flex gap-2">
      <Button onClick={() => toast.success({ title: "保存成功", body: "笔记已保存", timeout: 0 })}>
        成功通知
      </Button>
      <Button onClick={() => toast.error({ title: "保存失败", body: "请重试保存", timeout: 0 })}>
        失败通知
      </Button>
    </section>
  );
}
