import { Badge, Button, Form, FormField, Input, Textarea } from "@bixlab/mtone";
import { useState } from "react";

// Test-only composition: no demo route or generic control implementation ships.
export function T17Form() {
  const [mode, setMode] = useState<"ready" | "disabled" | "loading">("ready");
  const [invalid, setInvalid] = useState(false);
  const [submissions, setSubmissions] = useState(0);
  const [submittedTitle, setSubmittedTitle] = useState("");

  return (
    <main className="mx-auto max-w-xl p-6">
      <h1>T17 基础表单消费验收</h1>
      <Badge tone="green">测试夹具</Badge>
      <Form
        onSubmit={(event) => {
          event.preventDefault();
          setSubmittedTitle(String(new FormData(event.currentTarget).get("title")));
          setSubmissions((count) => count + 1);
        }}
      >
        <FormField
          label="标题"
          description="填写笔记标题"
          error={invalid ? "标题不可用" : undefined}
        >
          <Input name="title" />
        </FormField>
        <FormField label="说明" description="补充上下文">
          <Textarea name="description" />
        </FormField>
        <Button
          type="submit"
          disabled={mode === "disabled"}
          loading={mode === "loading"}
        >
          保存
        </Button>
      </Form>
      <div className="mt-3 flex gap-2">
        <Button onClick={() => setMode("disabled")}>禁用提交</Button>
        <Button onClick={() => setMode("loading")}>进入加载</Button>
        <Button onClick={() => setMode("ready")}>恢复提交</Button>
        <Button onClick={() => setInvalid((value) => !value)}>切换错误</Button>
      </div>
      <p role="status">提交次数：{submissions}；标题：{submittedTitle}</p>
    </main>
  );
}
