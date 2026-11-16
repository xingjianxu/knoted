// Knoted 测试实例编排的场景测试（pnpm test:pc）。
// 覆盖：pcwrap/编排配置形状、固定产物校验、模板渲染、证书校验、端口占用、迁移失败传播。
import { spawnSync } from "node:child_process";
import {
  chmodSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
  appendFileSync,
} from "node:fs";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { after, before, describe, it } from "node:test";
import assert from "node:assert/strict";

const repoRoot = resolve(import.meta.dirname, "../..");
const serviceScript = join(repoRoot, "tools/dev/pc/service.sh");
const checkCertificate = join(repoRoot, "tools/dev/pc/check-certificate.sh");
const revision = "0123456789abcdef0123456789abcdef01234567";

const temporary = [];

function makeTempDir(label) {
  const dir = mkdtempSync(join(tmpdir(), `knoted-pc-${label}-`));
  temporary.push(dir);
  return dir;
}

function run(command, args, options = {}) {
  return spawnSync(command, args, {
    encoding: "utf8",
    timeout: 60_000,
    ...options,
  });
}

async function freePort() {
  const server = createServer();
  await new Promise((resolveListen) => server.listen(0, "127.0.0.1", resolveListen));
  const { port } = server.address();
  await new Promise((resolveClose) => server.close(resolveClose));
  return port;
}

function writeExecutable(path, contents) {
  writeFileSync(path, contents);
  chmodSync(path, 0o755);
}

// 用与 tools/dev/build-artifact.sh 相同的命令生成清单，避免重新实现排序规则。
function writeManifest(root) {
  const result = run(
    "bash",
    [
      "-c",
      "find bin share -type f -print0 | sort -z | xargs -0 sha256sum > SHA256SUMS",
    ],
    { cwd: root, env: { ...process.env, LC_ALL: "C" } },
  );
  assert.equal(result.status, 0, `manifest fixture failed: ${result.stderr}`);
}

function createRuntimeFixture() {
  const root = join(makeTempDir("runtime"), "current");
  mkdirSync(join(root, "bin"), { recursive: true });
  mkdirSync(join(root, "share/knoted/web"), { recursive: true });
  mkdirSync(join(root, "share/knoted/migrations"), { recursive: true });
  writeExecutable(join(root, "bin/knoted-api"), "#!/usr/bin/env bash\nexit 0\n");
  writeFileSync(join(root, "share/knoted/web/index.html"), "<!doctype html><title>knoted</title>\n");
  writeFileSync(join(root, "share/knoted/migrations/0001_registry.sql"), "select 1;\n");
  writeFileSync(join(root, "share/knoted/REVISION"), `${revision}\n`);
  writeManifest(root);
  return root;
}

function createCertificateFixture(hostname) {
  const dir = makeTempDir("cert");
  const result = run("openssl", [
    "req",
    "-x509",
    "-newkey",
    "rsa:2048",
    "-nodes",
    "-days",
    "2",
    "-subj",
    `/CN=${hostname}`,
    "-addext",
    `subjectAltName=DNS:${hostname}`,
    "-keyout",
    join(dir, "key.pem"),
    "-out",
    join(dir, "fullchain.pem"),
  ]);
  assert.equal(result.status, 0, `openssl fixture failed: ${result.stderr}`);
  return dir;
}

function prepareEnv({ runtimeRoot, instanceRoot, certDir, apiPort, httpsPort, domain }) {
  return {
    ...process.env,
    KNOTED_INSTANCE_ROOT: instanceRoot,
    KNOTED_RUNTIME_ROOT: runtimeRoot,
    KNOTED_CERT_DIR: certDir,
    KNOTED_API_PORT: String(apiPort),
    KNOTED_HTTPS_PORT: String(httpsPort),
    KNOTED_HTTPS_DOMAIN: domain,
    KNOTED_LOG_LEVEL: "debug",
    DATABASE_URL: "postgres://dev@127.0.0.1:5432/dev",
  };
}

async function fullFixture() {
  const base = makeTempDir("instance");
  const runtimeRoot = createRuntimeFixture();
  const certDir = createCertificateFixture("knoted.example.test");
  const apiPort = await freePort();
  const httpsPort = await freePort();
  return {
    base,
    runtimeRoot,
    certDir,
    apiPort,
    httpsPort,
    domain: "knoted.example.test",
    instanceRoot: join(base, "service"),
  };
}

before(() => {
  for (const dir of temporary) rmSync(dir, { recursive: true, force: true });
});

after(() => {
  for (const dir of temporary) rmSync(dir, { recursive: true, force: true });
});

describe("编排配置", () => {
  it("pcwrap 固定控制端口、日志与项目名", () => {
    const config = JSON.parse(readFileSync(join(repoRoot, "pcwrap.config.json"), "utf8"));
    assert.equal(config.name, "knoted");
    assert.equal(config.port, 15600);
    assert.equal(config.configFile, "process-compose.yaml");
    assert.equal(config.logFile, "/data/knoted-dev/service/logs/process-compose.log");
    assert.equal(config.keepProject, true);
    assert.equal(config.profiles, undefined, "不定义 prod profile");
  });

  it("编排只含测试应用进程，控制端口不写进 yaml", () => {
    const yaml = readFileSync(join(repoRoot, "process-compose.yaml"), "utf8");
    for (const name of ["prepare", "migrate", "knoted-api", "knoted-https"]) {
      assert.match(yaml, new RegExp(`\\n  ${name}:`), `缺少进程 ${name}`);
    }
    assert.doesNotMatch(yaml, /^\s*port:/m, "process-compose.yaml 不接受 port 字段，控制端口由 pcwrap 注入");
    const processNames = yaml
      .split("\n")
      .filter((line) => /^  [a-z][a-z0-9-]*:$/.test(line))
      .map((line) => line.trim().replace(":", ""));
    assert.deepEqual(
      processNames.sort(),
      ["knoted-api", "knoted-https", "migrate", "prepare"],
      "外部依赖（PostgreSQL/Strata）不得进编排",
    );
    assert.match(yaml, /condition: process_completed_successfully/);
    assert.match(yaml, /condition: process_healthy/);
  });
});

describe("prepare", () => {
  it("渲染配置/Caddyfile 并安装稳定 reload 入口", async () => {
    const fixture = await fullFixture();
    const result = run(
      "bash",
      [serviceScript, "prepare"],
      { env: prepareEnv(fixture) },
    );
    assert.equal(result.status, 0, result.stderr);

    const toml = readFileSync(join(fixture.instanceRoot, "config/knoted.toml"), "utf8");
    assert.match(toml, new RegExp(`bind = "127\\.0\\.0\\.1:${fixture.apiPort}"`));
    assert.match(toml, /rp_id = "knoted\.example\.test"/);
    assert.match(toml, new RegExp(`origin = "https://knoted\\.example\\.test:${fixture.httpsPort}"`));
    assert.match(toml, /url = "postgres:\/\/dev@127\.0\.0\.1:5432\/dev"/);
    assert.doesNotMatch(toml, /@[A-Z_]+@/);

    const caddy = readFileSync(join(fixture.instanceRoot, "config/Caddyfile"), "utf8");
    assert.match(caddy, new RegExp(`https://knoted\\.example\\.test:${fixture.httpsPort} \\{`));
    assert.match(caddy, new RegExp(`X-Knoted-Revision ${revision}`));
    assert.match(caddy, new RegExp(`reverse_proxy 127\\.0\\.0\\.1:${fixture.apiPort}`));
    assert.match(caddy, new RegExp(`root \\* ${fixture.runtimeRoot}/share/knoted/web`));
    assert.match(caddy, /auto_https off/);
    assert.doesNotMatch(caddy, /@[A-Z_]+@/);

    assert.equal(statSync(join(fixture.instanceRoot, "config/knoted.toml")).mode & 0o777, 0o600);
    assert.equal(statSync(join(fixture.instanceRoot, "state")).mode & 0o777, 0o700);

    const reload = join(fixture.instanceRoot, "bin/knoted-https-reload");
    assert.equal(statSync(reload).mode & 0o777, 0o755);
    const reloadBody = readFileSync(reload, "utf8");
    assert.match(reloadBody, /flock 9/);
    assert.match(reloadBody, /caddy reload --config/);
    assert.match(reloadBody, new RegExp(fixture.certDir.replaceAll("/", "\\/")));
    assert.match(reloadBody, new RegExp(`unix/${fixture.instanceRoot}/state/https/admin.sock`));
  });

  it("拒绝被篡改或清单不完整的固定产物", async () => {
    const fixture = await fullFixture();
    appendFileSync(join(fixture.runtimeRoot, "share/knoted/web/index.html"), "<!-- tampered -->\n");
    const result = run("bash", [serviceScript, "prepare"], { env: prepareEnv(fixture) });
    assert.equal(result.status, 1);
    assert.match(result.stderr, /SHA256SUMS/);
    assert.equal(
      readdirSync(fixture.base).includes("service"),
      false,
      "校验失败时不应创建实例目录",
    );
  });

  it("证书缺失时拒绝启动", async () => {
    const fixture = await fullFixture();
    rmSync(join(fixture.certDir, "fullchain.pem"));
    const result = run("bash", [serviceScript, "prepare"], { env: prepareEnv(fixture) });
    assert.equal(result.status, 1);
    assert.match(result.stderr, /证书不可读/);
  });

  it("端口被占用时拒绝启动", async () => {
    const fixture = await fullFixture();
    const blocker = createServer();
    await new Promise((resolveListen) => blocker.listen(fixture.apiPort, "127.0.0.1", resolveListen));
    try {
      const result = run("bash", [serviceScript, "prepare"], { env: prepareEnv(fixture) });
      assert.equal(result.status, 1);
      assert.match(result.stderr, new RegExp(`端口 ${fixture.apiPort} 已被占用`));
    } finally {
      await new Promise((resolveClose) => blocker.close(resolveClose));
    }
  });
});

describe("证书校验", () => {
  it("接受匹配 hostname 的证书", () => {
    const certDir = createCertificateFixture("knoted.example.test");
    const result = run("bash", [
      checkCertificate,
      join(certDir, "fullchain.pem"),
      join(certDir, "key.pem"),
      "knoted.example.test",
    ]);
    assert.equal(result.status, 0, result.stderr);
    assert.equal(result.stdout, "", "成功时不打印证书内容");
  });

  it("拒绝 hostname 不匹配的证书", () => {
    const certDir = createCertificateFixture("knoted.example.test");
    const result = run("bash", [
      checkCertificate,
      join(certDir, "fullchain.pem"),
      join(certDir, "key.pem"),
      "wlab.example.test",
    ]);
    assert.equal(result.status, 1);
    assert.match(result.stderr, /hostname 不匹配/);
  });
});

describe("migrate", () => {
  it("把固定产物的 migrations 交给 sqlx，并传播失败退出码", async () => {
    const fixture = await fullFixture();
    run("bash", [serviceScript, "prepare"], { env: prepareEnv(fixture) });

    const stubDir = makeTempDir("stub");
    const record = join(stubDir, "args.txt");
    writeExecutable(
      join(stubDir, "sqlx"),
      `#!/usr/bin/env bash\nprintf '%s\\n' "$*" > ${JSON.stringify(record)}\nexit 0\n`,
    );
    const okEnv = { ...prepareEnv(fixture), PATH: `${stubDir}:${process.env.PATH}` };
    const ok = run("bash", [serviceScript, "migrate"], { env: okEnv });
    assert.equal(ok.status, 0, ok.stderr);
    assert.match(
      readFileSync(record, "utf8"),
      new RegExp(`migrate run --source ${fixture.runtimeRoot}/share/knoted/migrations`),
    );

    writeExecutable(join(stubDir, "sqlx"), "#!/usr/bin/env bash\nexit 7\n");
    const failed = run("bash", [serviceScript, "migrate"], { env: okEnv });
    assert.equal(failed.status, 7, "迁移失败必须原样传播退出码");
  });

  it("缺少 sqlx 时给出明确错误而不是静默跳过", async () => {
    const fixture = await fullFixture();
    run("bash", [serviceScript, "prepare"], { env: prepareEnv(fixture) });
    const result = run("bash", [serviceScript, "migrate"], { env: prepareEnv(fixture) });
    assert.equal(result.status, 1);
    assert.match(result.stderr, /sqlx-cli/);
  });
});

describe("paths", () => {
  it("默认指向本机测试实例", () => {
    const env = { ...process.env };
    for (const key of Object.keys(env)) {
      if (key.startsWith("KNOTED_") || key === "DATABASE_URL") delete env[key];
    }
    const result = run("bash", [serviceScript, "paths"], { env });
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout, /^instance_root=\/data\/knoted-dev\/service$/m);
    assert.match(result.stdout, /^api_port=15602$/m);
    assert.match(result.stdout, /^https_port=15601$/m);
    assert.match(result.stdout, /^domain=wlab\.132140\.xyz$/m);
    assert.match(result.stdout, /^admin_socket=\/data\/knoted-dev\/service\/state\/https\/admin\.sock$/m);
  });
});

describe("仓库入口", () => {
  it("package.json 暴露 pc 与 test:pc 脚本", () => {
    const pkg = JSON.parse(readFileSync(join(repoRoot, "package.json"), "utf8"));
    assert.equal(pkg.scripts.pc, "pcwrap");
    assert.match(pkg.scripts["test:pc"], /node --test/);
    assert.equal(pkg.devDependencies["@bixlab/pcwrap"], "0.1.0");
  });
});
