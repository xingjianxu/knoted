import { ListItem, Tag } from "@bixlab/mtone";
import { Link } from "@tanstack/react-router";
import { Fragment, type ReactNode } from "react";

import { searchTypes, type SearchHit } from "./queries";

function highlighted(text: string, query: string): ReactNode {
  const words = [...new Set(query.split(/\s+/u).filter(Boolean))];
  if (!words.length) return text;
  words.sort((left, right) => right.length - left.length);
  const pattern = words.map((word) => word.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")).join("|");
  return text.split(new RegExp(`(${pattern})`, "giu")).map((part, index) => index % 2 === 0
    ? <Fragment key={index}>{part}</Fragment>
    : <mark className="bg-accent-soft text-inherit" key={index}>{part}</mark>);
}

function resultTitle({ entity_type, id, title }: SearchHit) {
  const className = "page-link wrap-anywhere font-semibold";
  switch (entity_type) {
    case "note": return <Link className={className} to="/notes/$noteId" params={{ noteId: id }}>{title}</Link>;
    case "project": return <Link className={className} to="/projects/$projectId" params={{ projectId: id }}>{title}</Link>;
    case "task": return <Link className={className} to="/tasks/$taskId" params={{ taskId: id }}>{title}</Link>;
  }
}

const timeFormat = new Intl.DateTimeFormat("zh-CN", { dateStyle: "medium", timeStyle: "short" });

export function SearchResult({ hit, query }: { hit: SearchHit; query: string }) {
  const updated = new Date(hit.updated_at);
  return (
    <ListItem description={
      <span className="block space-y-1 wrap-anywhere">
        {hit.snippet && <span className="block whitespace-pre-wrap text-ink">{highlighted(hit.snippet, query)}</span>}
        <span className="flex flex-wrap gap-x-3 text-sm text-muted">
          {Number.isFinite(updated.getTime()) && <time dateTime={hit.updated_at}>{timeFormat.format(updated)}</time>}
          {hit.entity_type === "note" && <span>无法定位到原文位置。</span>}
        </span>
      </span>
    }>
      <span className="flex flex-wrap items-center gap-2">
        <Tag>{searchTypes[hit.entity_type]}</Tag>
        {resultTitle(hit)}
      </span>
    </ListItem>
  );
}
