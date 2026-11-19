// Forward real executions. Corrupt just the reported size of a selected completed batch.
import { existsSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import http from "node:http";
import path from "node:path";
import process from "node:process";
import { Buffer } from "node:buffer";

const run = process.env.T71_RUN;
const input = path.join(run, "staging/input");
const fault = path.join(run, "fail-embedding-term");

function selectedExecution(url) {
  const id = url.match(/^\/internal\/v1\/executions\/([a-f0-9-]{36})\/result$/)?.[1];
  if (!id || !existsSync(fault)) return;
  const filename = readdirSync(input).find(name => name.startsWith("embed-" + id + "-"));
  if (!filename) return;
  const term = readFileSync(fault, "utf8");
  const manifest = JSON.parse(readFileSync(path.join(input, filename), "utf8"));
  if (manifest.chunks.some(chunk => chunk.text.includes(term))) return id;
}

http.createServer((req, res) => {
  const upstream = http.request({
    hostname: "127.0.0.1", port: 17826, path: req.url, method: req.method, headers: req.headers,
  }, async reply => {
    const execution = reply.statusCode === 200 && req.method === "GET" && selectedExecution(req.url);
    if (execution) {
      const parts = [];
      for await (const part of reply) parts.push(part);
      const result = JSON.parse(Buffer.concat(parts).toString("utf8"));
      result.size = 8 * 1024 * 1024 + 1; // One byte beyond the worker's artifact limit.
      writeFileSync(path.join(run, "invalid-result-id"), execution);
      res.writeHead(200, { "Content-Type": "application/json" }).end(JSON.stringify(result));
      return;
    }
    res.writeHead(reply.statusCode, reply.headers);
    reply.pipe(res);
  });
  upstream.on("error", () => res.writeHead(502).end());
  req.pipe(upstream);
}).listen(17827, "127.0.0.1");
