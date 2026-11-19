// Keep reads live while deliberately failing document writes in the private index.
import { existsSync, writeFileSync } from "node:fs";
import http from "node:http";
import path from "node:path";
import process from "node:process";
import { URL } from "node:url";

const run = process.env.T71_RUN;
http.createServer((req, res) => {
  const pathname = new URL(req.url, "http://localhost").pathname;
  if (existsSync(path.join(run, "hold-writes")) &&
      /^(PUT|POST|DELETE)$/.test(req.method) &&
      /^\/indexes\/t71_documents\/documents(?:\/|$)/.test(pathname)) {
    writeFileSync(path.join(run, "write-refused"), "");
    res.writeHead(503).end();
    return;
  }
  const upstream = http.request({
    hostname: "127.0.0.1", port: 17824, path: req.url, method: req.method, headers: req.headers,
  }, reply => {
    res.writeHead(reply.statusCode, reply.headers);
    reply.pipe(res);
  });
  upstream.on("error", () => res.writeHead(502).end());
  req.pipe(upstream);
}).listen(17823, "127.0.0.1");
