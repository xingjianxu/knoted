import { useLayoutEffect, useReducer, useRef } from "react";
import { useQuery } from "@tanstack/react-query";
import { Alert, Button, Card, Empty, List, ListItem, PageLayout, Spinner, Status } from "@bixlab/mtone";
import type { QueryClientBundle } from "../../lib/query";
import { createCursorState, cursorReducer } from "../../lib/cursor-state";
import { filesQuery, type FileSource } from "./queries";
import { FileDetail } from "./FileDetail";

export function FileList({ items, selectedId, onSelectFile }: {
  items: FileSource[];
  selectedId?: string;
  onSelectFile: (id: string, trigger: HTMLButtonElement) => void;
}) {
  if (items.length === 0) return <Empty title="暂无文件" />;
  return <List aria-label="文件列表项">{items.map(file => {
    const present = file.status === "present";
    return <ListItem key={file.id} description={<span className="wrap-anywhere">{file.relative_path}</span>}>
      <div className="flex flex-wrap items-center gap-2">
        <Button variant="ghost" aria-label={`打开文件 ${file.name}`} aria-current={selectedId === file.id ? "page" : undefined}
          className="page-link h-auto max-w-full !whitespace-normal !p-0 text-start wrap-anywhere aria-[current=page]:font-semibold"
          onClick={event => onSelectFile(file.id, event.currentTarget)}>{file.name}</Button>
        <Status tone={present ? "green" : "orange"}>{present ? "存在" : "已丢失"}</Status>
      </div>
    </ListItem>;
  })}</List>;
}

export function FilesPage({ bundle, fileId, onSelectFile }: {
  bundle: QueryClientBundle;
  fileId?: string;
  onSelectFile: (id: string | null) => void;
}) {
  const [page, dispatch] = useReducer(cursorReducer, undefined, () => createCursorState("files"));
  const list = useQuery(filesQuery(bundle, page.cursor));
  const previousTrigger = useRef<HTMLButtonElement>(null);
  useLayoutEffect(() => {
    const trigger = previousTrigger.current;
    if (fileId || !trigger?.isConnected) return;
    const frame = window.requestAnimationFrame(() => {
      if (trigger.isConnected) {
        trigger.focus({ preventScroll: true });
        trigger.scrollIntoView({ block: "nearest" });
      }
      previousTrigger.current = null;
    });
    return () => window.cancelAnimationFrame(frame);
  }, [fileId]);

  const listContent = (
    <div className="space-y-3">
      {list.isPending && <Spinner label="正在读取文件列表" />}
      {list.error && <Alert tone="error" title={list.error.message}><Button onClick={() => void list.refetch()}>重试文件列表</Button></Alert>}
      {list.data && (list.data.items.length > 0 || (!list.isFetching && !list.isError)) && <FileList items={list.data.items} selectedId={fileId} onSelectFile={(id, trigger) => {
        previousTrigger.current = trigger;
        onSelectFile(id);
      }} />}
      {(list.data || page.previous.length > 0) && <nav aria-label="文件翻页" className="flex flex-wrap gap-2">
        <Button disabled={list.isFetching || !page.previous.length} onClick={() => dispatch({ type: "previous" })}>上一页文件</Button>
        <Button disabled={list.isFetching || list.isError || !list.data?.next_cursor || list.data.next_cursor === page.cursor}
          onClick={() => dispatch({ type: "next", nextCursor: list.data?.next_cursor ?? null })}>下一页文件</Button>
      </nav>}
    </div>
  );

  return <div className={fileId ? "grid gap-5 @min-[62rem]/workspace:grid-cols-[minmax(0,17rem)_minmax(0,1fr)]" : undefined}>
    {fileId && <div className="min-w-0 @min-[62rem]/workspace:col-start-2 @min-[62rem]/workspace:row-start-1">
      <Card className="min-w-0">
        <FileDetail key={fileId} bundle={bundle} fileId={fileId} onSelectFile={onSelectFile} />
      </Card>
    </div>}
    <section aria-label="文件列表" className={fileId ? "hidden min-w-0 @min-[62rem]/workspace:col-start-1 @min-[62rem]/workspace:row-start-1 @min-[62rem]/workspace:block" : undefined}>
      <PageLayout title="文件" headingLevel={fileId ? 2 : 1}>{listContent}</PageLayout>
    </section>
  </div>;
}
