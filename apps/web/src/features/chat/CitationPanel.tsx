import { Alert, Badge, Button, Empty, List, ListItem, Spinner } from "@bixlab/mtone";
import { useInfiniteQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useState } from "react";
import type { QueryClientBundle } from "../../lib/query";
import { citationsQuery, type Citation } from "./queries";

const sourceStates = {
  current: null,
  archived: "来源已归档",
  deleted: "来源已删除",
  missing: "来源不可用",
} satisfies Record<Citation["source_state"], string | null>;

function CitationTitle({ citation }: { citation: Citation }) {
  if (citation.source_state !== "current") return <span>{citation.title}</span>;
  const className = "text-accent hover:underline";
  switch (citation.source_type) {
    case "note":
      return <Link className={className} to="/notes/$noteId" params={{ noteId: citation.source_id }}>{citation.title}</Link>;
    case "file":
      return <Link className={className} to="/files/$fileId" params={{ fileId: citation.source_id }}>{citation.title}</Link>;
    case "task":
      return <Link className={className} to="/tasks/$taskId" params={{ taskId: citation.source_id }}>{citation.title}</Link>;
  }
}

export function CitationList({ citations }: { citations: Citation[] }) {
  return (
    <List aria-label="引用来源">
      {citations.map((citation) => (
        <ListItem
          key={citation.id}
          description={
            <span className="block space-y-1 wrap-anywhere">
              {citation.location && <span className="block text-muted">{citation.location}</span>}
              <span className="block whitespace-pre-wrap text-ink">{citation.snippet}</span>
            </span>
          }
        >
          <span className="flex flex-wrap items-center gap-2">
            <CitationTitle citation={citation} />
            {sourceStates[citation.source_state] && <Badge tone="orange">{sourceStates[citation.source_state]}</Badge>}
          </span>
        </ListItem>
      ))}
    </List>
  );
}

function SavedCitations({ bundle, messageId }: { bundle: QueryClientBundle; messageId: string }) {
  const query = useInfiniteQuery(citationsQuery(bundle, messageId));
  const citations = query.data?.pages.flatMap((page) => page.items) ?? [];
  return (
    <div className="space-y-2">
      {query.isPending && <Spinner label="正在读取引用" />}
      {query.isError && <Alert tone="error">
        <span>无法读取引用。</span>{" "}
        <Button onClick={() => void query.refetch()}>重试</Button>
      </Alert>}
      {citations.length > 0 && <CitationList citations={citations} />}
      {query.isSuccess && citations.length === 0 && <Empty title="没有引用" />}
      {query.hasNextPage && <Button disabled={query.isFetching} onClick={() => void query.fetchNextPage()}>更多引用</Button>}
    </div>
  );
}

export function CitationPanel({ bundle, messageId }: { bundle: QueryClientBundle; messageId: string }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="space-y-2">
      <Button aria-expanded={open} onClick={() => setOpen(!open)} variant="ghost">
        {open ? "收起引用" : "查看引用"}
      </Button>
      {open && <SavedCitations bundle={bundle} messageId={messageId} />}
    </div>
  );
}
