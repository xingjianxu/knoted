// dev 一键部署脚本的场景测试（pnpm test:pc）。
// 覆盖：迁移判定驱动 down+up 或 restart、已最新空转、构建失败不动指针、
// 构建前必须拿到运行变量、切换后验证失败复位指针、deploy.env 抓取与权限、
// 缺变量在切换前退出、.worktrees 路径守卫。
//
// 脚本不复制 build-artifact.sh / pcwrap / service.sh 的逻辑，测试因此用
// PATH 前置 stub（pnpm、curl）与显式 seam（产物父目录、/proc 根、构建命令）
// 模拟真实副作用，而不是等待一次几分钟的真实构建。
import { spawnSync } from "node:child_process";
import {
  appendFileSync,
  chmodSync,
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { resolve } from "node:path";
import { after, describe, it } from "node:test";
import assert from "node:assert/strict";

const repoRoot = resolve(import.meta.dirname, "../..");
const deployScript = join(repoRoot, "tools/dev/deploy-dev.sh");
const targetRevision = spawnSync("git", ["rev-parse", "HEAD"], {
  cwd: repoRoot,
  encoding: "utf8",
}).stdout.trim();
const previousRevision = "0".repeat(39) + "1";

const temporary = [];
after(() => {
  for (const dir of temporary) rmSync(dir, { recursive: true, force: true });
});

function makeTempDir(label) {
  const dir = mkdtempSync(join(tmpdir(), `knoted-deploy-${label}-`));
  temporary.push(dir);
  return dir;
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    encoding: "utf8",
    timeout: 120_000,
    ...options,
  });
  if (result.error) {
    return {
      ...result,
      status: -1,
      stdout: result.stdout ?? "",
      stderr: `${command}: ${result.error.message}`,
    };
  }
  return result;
}

function writeExecutable(path, contents) {
  mkdirSync(resolve(path, ".."), { recursive: true });
  writeFileSync(path, contents);
  chmodSync(path, 0o755);
}

// 与 tools/dev/build-artifact.sh 相同的命令生成清单，避免测试自己实现排序规则。
function writeManifest(root) {
  const result = run("bash", ["-c", "find bin share -type f -print0 | sort -z | xargs -0 sha256sum > SHA256SUMS"], {
    cwd: root,
    env: { ...process.env, LC_ALL: "C" },
  });
  assert.equal(result.status, 0, `manifest fixture failed: ${result.stderr}`);
}

// 固定产物最小形状，供部署脚本及真实 prepare 验证。
function createArtifact(parent, revision, migrations = ["0001_registry.sql"]) {
  const root = join(parent, revision);
  mkdirSync(join(root, "bin"), { recursive: true });
  mkdirSync(join(root, "share/knoted/web"), { recursive: true });
  mkdirSync(join(root, "share/knoted/migrations"), { recursive: true });
  mkdirSync(join(root, "share/knoted/intelligence/python/bin"), { recursive: true });
  for (const name of ["knoted-api", "knoted-worker", "knoted-intelligence"]) {
    writeExecutable(join(root, `bin/${name}`), '#!/usr/bin/env bash\nprintf "%s" "${KNOTED_DEV_CHAT_CONFIG_DIGEST:-}"\n');
  }
  writeExecutable(
    join(root, "share/knoted/intelligence/python/bin/python3.12"),
    '#!/usr/bin/env bash\nexec python3 "$@"\n',
  );
  for (const required of [
    "BUILD.json", "requirements.txt", "uv.lock", "python/lib/python3.12/os.py",
    ...[
      "knoted_intelligence/app.py", "knoted_intelligence/guardian.py", "knoted_intelligence/supervision.py",
      "knoted_intelligence/parsing/pdf.py", "docling/document_converter.py",
      "torch/__init__.py", "onnxruntime/__init__.py",
    ].map(name => `python/lib/python3.12/site-packages/${name}`),
  ]) {
    const path = join(root, "share/knoted/intelligence", required);
    mkdirSync(resolve(path, ".."), { recursive: true });
    writeFileSync(path, "fixture\n");
  }
  writeFileSync(join(root, "share/knoted/web/index.html"), "<!doctype html><title>knoted</title>\n");
  for (const migration of migrations) {
    writeFileSync(join(root, `share/knoted/migrations/${migration}`), "select 1;\n");
  }
  writeFileSync(join(root, "share/knoted/REVISION"), `${revision}\n`);
  writeManifest(root);
  return root;
}

// 真实实例里 api/worker 直接跑 Rust 二进制，intelligence 的 bash 包装器 exec 固定解释器，
// 所以 /proc/exe 看到的是产物里的 python3.12。
const processImages = {
  "knoted-api": "bin/knoted-api",
  "knoted-worker": "bin/knoted-worker",
  "knoted-intelligence": "share/knoted/intelligence/python/bin/python3.12",
};

function createProcFixture(procRoot, artifactRoot) {
  Object.values(processImages).forEach((image, index) => {
    mkdirSync(join(procRoot, String(1000 + index)), { recursive: true });
    symlinkSync(join(artifactRoot, image), join(procRoot, String(1000 + index), "exe"));
    if (index < 2) writeFileSync(join(procRoot, String(1000 + index), "environ"), "KNOTED_DEV_CHAT_CONFIG_DIGEST=none\0");
  });
  // 生产实例跑在 Nix store 里，必须不被 dev 部署的进程校验误伤。
  mkdirSync(join(procRoot, "2000"), { recursive: true });
  symlinkSync("/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-knoted/bin/knoted-api", join(procRoot, "2000", "exe"));
}

// stub 的 pnpm 记录调用顺序，并在"重启"时把假 /proc 的可执行文件指向当前
// current 指针，模拟 process-compose 真的换了进程映像。`process list` 是只读探测，
// 不计入调用记录。
function writePnpmStub(path) {
  writeExecutable(
    path,
    `#!/usr/bin/env bash
set -euo pipefail
relink() {
  local target index pid image
  [ "\${STUB_NO_RELINK:-0}" = 1 ] && return 0
  target=$(readlink -f "\${STUB_RUNTIME_PARENT:?}/current")
  index=0
  for image in bin/knoted-api bin/knoted-worker share/knoted/intelligence/python/bin/python3.12; do
    pid=$((1000 + index))
    rm -f "\${STUB_PROC:?}/$pid/exe"
    ln -s "$target/$image" "\${STUB_PROC:?}/$pid/exe"
    index=$((index + 1))
  done
}
case "$*" in
"pc process list")
  [[ \${STUB_INSTANCE_STOPPED:-0} != 1 ]] || exit 1
  exit 0
  ;;
esac
printf '%s|%s\\n' "$PWD" "$*" >>"\${STUB_LOG:?}"
case "$*" in
"pc down")
  [ "\${STUB_DOWN_FAIL:-0}" = 1 ] && exit 1
  exit 0
  ;;
"pc process restart "*)
  [ "\${STUB_RESTART_FAIL:-0}" = 1 ] && exit 1
  relink
  exit 0
  ;;
"pc up -D")
  relink
  if [[ \${STUB_SKIP_CHAT_CONFIG:-0} != 1 ]]; then
    export KNOTED_RUNTIME_ROOT="\${STUB_RUNTIME_PARENT:?}/current"
    bash "\${STUB_SERVICE_SCRIPT:?}" prepare
    if [[ \${STUB_SKIP_CHAT_MARKER:-0} != 1 ]]; then
      marker=$(bash "$STUB_SERVICE_SCRIPT" api)
      printf 'KNOTED_DEV_CHAT_CONFIG_DIGEST=%s\\0' "$marker" > "$STUB_PROC/1000/environ"
      marker=$(bash "$STUB_SERVICE_SCRIPT" worker)
      printf 'KNOTED_DEV_CHAT_CONFIG_DIGEST=%s\\0' "$marker" > "$STUB_PROC/1001/environ"
    fi
  fi
  exit 0
  ;;
esac
exit 0
`,
  );
}

// stub 的 curl 覆盖 readiness 探针与 HTTPS 验证，两个开关分开注入：
// STUB_READY_FAIL 模拟实例没起来，STUB_HTTPS_FAIL 模拟进程就绪但入口不通。
function writeCurlStub(path) {
  writeExecutable(
    path,
    `#!/usr/bin/env bash
set -euo pipefail
for argument in "$@"; do
  case "$argument" in
  *internal/v1/health*)
    [ "\${STUB_READY_FAIL:-0}" = 1 ] && exit 22
    printf '{"status":"ok"}'
    exit 0
    ;;
  *health/ready*)
    [ "\${STUB_READY_FAIL:-0}" = 1 ] && exit 22
    printf '{"status":"ready"}'
    exit 0
    ;;
  https://*)
    [ "\${STUB_HTTPS_FAIL:-0}" = 1 ] && exit 22
    printf '200'
    exit 0
    ;;
  esac
done
exit 0
`,
  );
}

function writeBuildStub(path) {
  writeExecutable(
    path,
    `#!/usr/bin/env bash
set -euo pipefail
printf '%s|build %s %s %s\\n' "$PWD" "$1" "$2" "\${KNOTED_OFFICE_ORIGIN:-}" >>"\${STUB_LOG:?}"
[ "\${STUB_BUILD_FAIL:-0}" = 1 ] && exit 1
cp -r "\${STUB_ARTIFACT_TEMPLATE:?}" "$2"
`,
  );
}

const officeOrigin = "https://knoted.example.test:15402";

function fixture({
  targetMigrations,
  previousMigrations = ["0001_registry.sql"],
  withTargetArtifact = true,
  runtimeParent,
} = {}) {
  const base = makeTempDir("run");
  const runtimeParentDir = runtimeParent ?? join(base, "runtime");
  mkdirSync(runtimeParentDir, { recursive: true });
  const instanceRoot = join(base, "service");
  mkdirSync(instanceRoot, { recursive: true });
  const procRoot = join(base, "proc");
  mkdirSync(procRoot, { recursive: true });
  const stubBin = join(base, "stubs");
  mkdirSync(stubBin, { recursive: true });
  const logFile = join(base, "pnpm.log");
  writeFileSync(logFile, "");

  const previousRoot = createArtifact(runtimeParentDir, previousRevision, previousMigrations);
  symlinkSync(previousRoot, join(runtimeParentDir, "current"));
  createProcFixture(procRoot, previousRoot);

  const templateRoot = join(base, "artifact-template");
  cpSync(createArtifact(join(base, "template-build"), targetRevision, targetMigrations), templateRoot, {
    recursive: true,
  });
  if (withTargetArtifact) createArtifact(runtimeParentDir, targetRevision, targetMigrations);

  writePnpmStub(join(stubBin, "pnpm"));
  writeCurlStub(join(stubBin, "curl"));
  const buildStub = join(base, "build-artifact-stub.sh");
  writeBuildStub(buildStub);

  const env = {
    ...process.env,
    PATH: `${stubBin}:${process.env.PATH}`,
    KNOTED_INSTANCE_ROOT: instanceRoot,
    KNOTED_DEPLOY_RUNTIME_PARENT: runtimeParentDir,
    KNOTED_DEPLOY_PROC_ROOT: procRoot,
    KNOTED_DEPLOY_BUILD_CMD: buildStub,
    KNOTED_DEPLOY_READY_TIMEOUT: "3",
    KNOTED_API_PORT: "15602",
    KNOTED_INTELLIGENCE_PORT: "15603",
    KNOTED_HTTPS_PORT: "15601",
    KNOTED_HTTPS_DOMAIN: "knoted.example.test",
    STUB_LOG: logFile,
    STUB_RUNTIME_PARENT: runtimeParentDir,
    STUB_PROC: procRoot,
    STUB_ARTIFACT_TEMPLATE: templateRoot,
    STUB_SERVICE_SCRIPT: join(repoRoot, "tools/dev/pc/service.sh"),
    KNOTED_ALLOW_BUSY_PORTS: "1",
    KNOTED_BUILD_ROOT: join(base, "build"),
    // 运行变量必须由 deploy.env 或活动进程提供，不能从测试宿主环境串味。
    KNOTED_INTELLIGENCE_DOCLING_ARTIFACTS_DIR: "",
    KNOTED_INTELLIGENCE_BGE_M3_DIR: "",
    KNOTED_SEARCH_ENABLED: "",
    KNOTED_VECTORS_ENABLED: "",
    KNOTED_OFFICE_ORIGIN: "",
    KNOTED_OFFICE_JWT_SECRET_FILE: "",
    KNOTED_CHAT_LOCAL_BASE_URL: "",
    KNOTED_CHAT_LOCAL_MODEL: "",
    KNOTED_CHAT_LOCAL_CREDENTIAL_FILE: "",
  };
  return { base, instanceRoot, runtimeParentDir, procRoot, logFile, env, templateRoot };
}

function deploy(context, args = [targetRevision]) {
  return run("bash", [deployScript, ...args], { cwd: repoRoot, env: context.env });
}

function pcCalls(logFile) {
  return readFileSync(logFile, "utf8").trim().split("\n").filter(Boolean);
}

// stub 逐行记录 “<cwd>|<argv>”，调用序列断言只看 argv。
function pcArgs(logFile) {
  return pcCalls(logFile).map((line) => line.split("|").slice(1).join("|"));
}

function pcCwd(logFile) {
  return [...new Set(pcCalls(logFile).map((line) => line.split("|")[0]))];
}

function mainCheckout() {
  const commonDir = run("git", ["rev-parse", "--path-format=absolute", "--git-common-dir"], {
    cwd: repoRoot,
    encoding: "utf8",
  }).stdout.trim();
  return commonDir.replace(/\/\.git$/, "");
}

function currentTarget(context) {
  return run("readlink", ["-f", join(context.runtimeParentDir, "current")]).stdout.trim();
}

function writeDeployEnv(instanceRoot, values) {
  const body = Object.entries(values)
    .map(([key, value]) => `${key}=${value}`)
    .join("\n");
  writeFileSync(join(instanceRoot, "deploy.env"), `${body}\n`, { mode: 0o600 });
}

const runtimeVariables = {
  KNOTED_INTELLIGENCE_DOCLING_ARTIFACTS_DIR: "/data/llm/docling/run",
  KNOTED_INTELLIGENCE_BGE_M3_DIR: "/data/llm/bge-m3/run",
  KNOTED_SEARCH_ENABLED: "1",
  KNOTED_VECTORS_ENABLED: "1",
  KNOTED_OFFICE_ORIGIN: officeOrigin,
  KNOTED_OFFICE_JWT_SECRET_FILE: "/var/lib/onlyoffice-secrets/jwt",
};

function useCurrentRevision(context) {
  const target = join(context.runtimeParentDir, targetRevision);
  rmSync(join(context.runtimeParentDir, "current"));
  symlinkSync(target, join(context.runtimeParentDir, "current"));
  rmSync(context.procRoot, { recursive: true, force: true });
  mkdirSync(context.procRoot, { recursive: true });
  createProcFixture(context.procRoot, target);
}

function modelVariables(context) {
  const credential = join(context.instanceRoot, "model-credential=");
  writeFileSync(credential, "synthetic\n", { mode: 0o600 });
  return {
    KNOTED_CHAT_LOCAL_BASE_URL: "http://127.0.0.1:15604",
    KNOTED_CHAT_LOCAL_MODEL: "fixture-model",
    KNOTED_CHAT_LOCAL_CREDENTIAL_FILE: credential,
  };
}

describe("deploy-dev.sh", () => {
  it("新迁移触发 down + up -D 并切换指针", () => {
    const context = fixture({ targetMigrations: ["0001_registry.sql", "0002_notes.sql"] });
    writeDeployEnv(context.instanceRoot, runtimeVariables);

    const result = deploy(context);
    assert.equal(result.status, 0, result.stderr);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, targetRevision));
    assert.deepEqual(pcArgs(context.logFile), ["pc down", "pc up -D"]);
  });

  it("迁移集合不变只重启三个进程", () => {
    const context = fixture({ targetMigrations: ["0001_registry.sql"] });
    writeDeployEnv(context.instanceRoot, runtimeVariables);

    const result = deploy(context);
    assert.equal(result.status, 0, result.stderr);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, targetRevision));
    assert.deepEqual(pcArgs(context.logFile), [
      "pc process restart knoted-api",
      "pc process restart knoted-worker",
      "pc process restart knoted-intelligence",
    ]);
    // pcwrap daemon 的 cwd 决定后续 prepare/restart 用哪份 service.sh，必须锚在主检出。
    assert.deepEqual(pcCwd(context.logFile), [mainCheckout()]);
  });

  it("停止失败时不切指针也不启动", () => {
    const context = fixture({ targetMigrations: ["0001_registry.sql", "0002_notes.sql"] });
    writeDeployEnv(context.instanceRoot, runtimeVariables);
    context.env.STUB_DOWN_FAIL = "1";

    const result = deploy(context);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /停止实例失败/);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, previousRevision));
    assert.deepEqual(pcArgs(context.logFile), ["pc down"]);
  });

  it("已是最新提交时不构建不重启", () => {
    const context = fixture({ targetMigrations: ["0001_registry.sql"] });
    useCurrentRevision(context);
    writeDeployEnv(context.instanceRoot, runtimeVariables);

    const result = deploy(context);
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(pcArgs(context.logFile), []);
    assert.match(result.stderr, /已是最新/);
  });

  it("同 SHA 部署重载模型，重复部署不重启", () => {
    const context = fixture();
    useCurrentRevision(context);
    writeDeployEnv(context.instanceRoot, { ...runtimeVariables, ...modelVariables(context) });
    const first = deploy(context);
    assert.equal(first.status, 0, first.stderr);
    assert.deepEqual(pcArgs(context.logFile), ["pc down", "pc up -D"]);
    const conf = join(context.instanceRoot, "config/knoted.toml");
    assert.match(readFileSync(conf, "utf8"), /^model = "fixture-model"$/m);

    writeFileSync(context.logFile, "");
    const second = deploy(context);
    assert.equal(second.status, 0, second.stderr);
    assert.deepEqual(pcArgs(context.logFile), []);
  });

  it("迁移不变但模型变更时仍重新运行 prepare", () => {
    const context = fixture();
    writeDeployEnv(context.instanceRoot, { ...runtimeVariables, ...modelVariables(context) });
    const result = deploy(context);
    assert.equal(result.status, 0, result.stderr);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, targetRevision));
    assert.deepEqual(pcArgs(context.logFile), ["pc down", "pc up -D"]);
  });

  it("凭据无法读取时在写 deploy.env 或停止实例前失败", () => {
    const context = fixture();
    useCurrentRevision(context);
    writeDeployEnv(context.instanceRoot, runtimeVariables);
    Object.assign(context.env, modelVariables(context));
    rmSync(context.env.KNOTED_CHAT_LOCAL_CREDENTIAL_FILE);
    const result = deploy(context);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /KNOTED_CHAT_LOCAL_CREDENTIAL_FILE.*读取/);
    assert.doesNotMatch(readFileSync(join(context.instanceRoot, "deploy.env"), "utf8"), /KNOTED_CHAT_LOCAL_/);
    assert.deepEqual(pcArgs(context.logFile), []);
  });

  it("健康端点通过但本地模型配置未应用时不报告部署成功", () => {
    const context = fixture();
    useCurrentRevision(context);
    writeDeployEnv(context.instanceRoot, { ...runtimeVariables, ...modelVariables(context) });
    context.env.STUB_SKIP_CHAT_CONFIG = "1";
    const result = deploy(context);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /chat\.local.*未应用/);
  });

  it("磁盘配置已更新但 Worker 未加载时也重新启动", () => {
    const context = fixture();
    useCurrentRevision(context);
    const vars = modelVariables(context);
    writeDeployEnv(context.instanceRoot, { ...runtimeVariables, ...vars });
    mkdirSync(join(context.instanceRoot, "config"));
    writeFileSync(join(context.instanceRoot, "config/knoted.toml"),
      `[chat.local]\nbase_url = "${vars.KNOTED_CHAT_LOCAL_BASE_URL}"\nmodel = "fixture-model"\ncredential_file = "${vars.KNOTED_CHAT_LOCAL_CREDENTIAL_FILE}"\n`);
    writeFileSync(join(context.procRoot, "1001/environ"), "KNOTED_DEV_CHAT_CONFIG_DIGEST=none\0");
    const result = deploy(context);
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(pcArgs(context.logFile), ["pc down", "pc up -D"]);
  });

  it("禁用聊天时也不把无摘要的旧进程误认为已应用", () => {
    const context = fixture();
    useCurrentRevision(context);
    writeDeployEnv(context.instanceRoot, runtimeVariables);
    writeFileSync(join(context.procRoot, "1000/environ"), "KNOTED_CHAT_LOCAL_MODEL=previous-model\0");
    const result = deploy(context);
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(pcArgs(context.logFile), ["pc down", "pc up -D"]);
  });

  it("无效的非聊天设置也在停止实例之前拒绝", () => {
    const context = fixture();
    useCurrentRevision(context);
    writeDeployEnv(context.instanceRoot, { ...runtimeVariables, ...modelVariables(context) });
    context.env.KNOTED_SEARCH_ENABLED = "oops";
    const result = deploy(context);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /KNOTED_SEARCH_ENABLED/);
    assert.deepEqual(pcArgs(context.logFile), []);
    assert.match(readFileSync(join(context.instanceRoot, "deploy.env"), "utf8"), /^KNOTED_SEARCH_ENABLED=1$/m);
  });

  it("模板渲染会拒绝的设置在停止实例前失败", () => {
    const context = fixture();
    useCurrentRevision(context);
    writeDeployEnv(context.instanceRoot, { ...runtimeVariables, ...modelVariables(context) });
    context.env.KNOTED_CHAT_LOCAL_MODEL = "fixture@MODEL@";
    const result = deploy(context);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /未替换占位符/);
    assert.deepEqual(pcArgs(context.logFile), []);
  });

  it("无法发布配置文件时不停止现有实例", () => {
    const context = fixture();
    useCurrentRevision(context);
    writeDeployEnv(context.instanceRoot, runtimeVariables);
    Object.assign(context.env, modelVariables(context));
    writeExecutable(join(context.base, "stubs/mv"), '#!/usr/bin/env bash\n[[ ${!#} != */deploy.env ]] || exit 1\nexec ' + run("which", ["mv"]).stdout.trim() + ' "$@"\n');
    const result = deploy(context);
    assert.notEqual(result.status, 0);
    assert.deepEqual(pcArgs(context.logFile), []);
  });

  it("删去文件中的模型配置时不从旧进程恢复", () => {
    const context = fixture();
    useCurrentRevision(context);
    const vars = modelVariables(context);
    writeDeployEnv(context.instanceRoot, { ...runtimeVariables, ...vars });
    const first = deploy(context);
    assert.equal(first.status, 0, first.stderr);
    appendFileSync(join(context.procRoot, "1000/environ"), Object.entries(vars).map(([k,v]) => `${k}=${v}\0`).join(""));
    writeFileSync(context.logFile, "");
    writeDeployEnv(context.instanceRoot, runtimeVariables);
    const result = deploy(context);
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(pcArgs(context.logFile), ["pc down", "pc up -D"]);
    assert.doesNotMatch(readFileSync(join(context.instanceRoot, "deploy.env"), "utf8"), /KNOTED_CHAT_LOCAL_/);
    assert.doesNotMatch(readFileSync(join(context.instanceRoot, "config/knoted.toml"), "utf8"), /chat\.local/);
  });

  for (const sameRevision of [true, false]) {
    it(`${sameRevision ? "同 SHA" : "不同 SHA 无迁移"}重载验证失败时恢复此前配置`, () => {
      const context = fixture();
      if (sameRevision) useCurrentRevision(context);
      const previous = currentTarget(context);
      const vars = modelVariables(context);
      writeDeployEnv(context.instanceRoot, { ...runtimeVariables, ...vars });
      if (sameRevision) {
        const first = deploy(context);
        assert.equal(first.status, 0, first.stderr);
      } else {
        const prepared = run("bash", [context.env.STUB_SERVICE_SCRIPT, "prepare"], {
          env: { ...context.env, ...runtimeVariables, ...vars, KNOTED_RUNTIME_ROOT: previous },
        });
        assert.equal(prepared.status, 0, prepared.stderr);
      }
      writeFileSync(context.logFile, "");
      writeDeployEnv(context.instanceRoot, {
        ...runtimeVariables, ...vars, KNOTED_CHAT_LOCAL_MODEL: "candidate-model", KNOTED_VECTORS_ENABLED: "0",
      });
      context.env.STUB_HTTPS_FAIL = "1";
      const result = deploy(context);
      assert.notEqual(result.status, 0);
      assert.equal(currentTarget(context), previous);
      assert.deepEqual(pcArgs(context.logFile), ["pc down", "pc up -D", "pc down", "pc up -D"]);
      const saved = readFileSync(join(context.instanceRoot, "deploy.env"), "utf8");
      assert.match(saved, /^KNOTED_CHAT_LOCAL_MODEL=fixture-model$/m);
      assert.match(saved, /^KNOTED_VECTORS_ENABLED=1$/m);
      assert.ok(saved.includes(`KNOTED_CHAT_LOCAL_CREDENTIAL_FILE=${vars.KNOTED_CHAT_LOCAL_CREDENTIAL_FILE}\n`));
      assert.match(readFileSync(join(context.instanceRoot, "config/knoted.toml"), "utf8"), /^model = "fixture-model"$/m);
    });
  }

  it("同 SHA 部署可以从持久配置启动已停止实例", () => {
    const context = fixture();
    useCurrentRevision(context);
    writeDeployEnv(context.instanceRoot, { ...runtimeVariables, ...modelVariables(context) });
    context.env.STUB_INSTANCE_STOPPED = "1";
    const result = deploy(context);
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(pcArgs(context.logFile), ["pc up -D"]);
  });

  it("启动后模型配置未被 Worker 加载则报告失败", () => {
    const context = fixture();
    useCurrentRevision(context);
    writeDeployEnv(context.instanceRoot, { ...runtimeVariables, ...modelVariables(context) });
    context.env.STUB_SKIP_CHAT_MARKER = "1";
    const result = deploy(context);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /未加载.*chat\.local/);
  });

  it("构建失败时指针不动", () => {
    const context = fixture({ targetMigrations: ["0001_registry.sql"], withTargetArtifact: false });
    writeDeployEnv(context.instanceRoot, runtimeVariables);
    context.env.STUB_BUILD_FAIL = "1";

    const result = deploy(context);
    assert.notEqual(result.status, 0);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, previousRevision));
    assert.deepEqual(
      pcArgs(context.logFile).filter((line) => line.startsWith("pc ")),
      [],
    );
  });

  it("构建时把固定提交、产物目录与 Office origin 传下去", () => {
    const context = fixture({ targetMigrations: ["0001_registry.sql"], withTargetArtifact: false });
    writeDeployEnv(context.instanceRoot, runtimeVariables);

    const result = deploy(context);
    assert.equal(result.status, 0, result.stderr);
    const buildCall = pcArgs(context.logFile).find((line) => line.startsWith("build "));
    assert.equal(buildCall, `build ${targetRevision} ${join(context.runtimeParentDir, targetRevision)} ${officeOrigin}`);
  });

  it("HTTPS 验证失败时把指针复位并重启回旧版本", () => {
    const context = fixture({ targetMigrations: ["0001_registry.sql"] });
    writeDeployEnv(context.instanceRoot, runtimeVariables);
    // 只让 HTTPS 失败：readiness 与进程映像都正常，真正走到 check_https。
    context.env.STUB_HTTPS_FAIL = "1";

    const result = deploy(context);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /HTTPS 入口不可用/);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, previousRevision));
    // 前三次是切到新产物的重启，之后必须再按旧指针重启一次。
    assert.deepEqual(pcArgs(context.logFile), [
      "pc process restart knoted-api",
      "pc process restart knoted-worker",
      "pc process restart knoted-intelligence",
      "pc process restart knoted-api",
      "pc process restart knoted-worker",
      "pc process restart knoted-intelligence",
    ]);
  });

  it("进程仍在旧目录时收敛等待会等满超时并复位", () => {
    const context = fixture({ targetMigrations: ["0001_registry.sql"] });
    writeDeployEnv(context.instanceRoot, runtimeVariables);
    // 重启“成功”但进程映像没换：只有 check_processes 能发现。
    context.env.STUB_NO_RELINK = "1";

    const result = deploy(context);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /仍运行在/);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, previousRevision));
  });

  it("带迁移的部署 readiness 超时时保留新产物而不是回退指针", () => {
    const context = fixture({ targetMigrations: ["0001_registry.sql", "0002_notes.sql"] });
    writeDeployEnv(context.instanceRoot, runtimeVariables);
    context.env.STUB_READY_FAIL = "1";

    const result = deploy(context);
    assert.notEqual(result.status, 0);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, targetRevision));
    assert.deepEqual(pcArgs(context.logFile), ["pc down", "pc up -D"]);
  });

  it("已存在但摘要不符的产物被移到旁路后重建", () => {
    const context = fixture({ targetMigrations: ["0001_registry.sql"] });
    writeDeployEnv(context.instanceRoot, runtimeVariables);
    const target = join(context.runtimeParentDir, targetRevision);
    appendFileSync(join(target, "bin/knoted-api"), "# corrupt\n");

    const result = deploy(context);
    assert.equal(result.status, 0, result.stderr);
    const quarantined = readdirSync(context.runtimeParentDir).filter((name) => name.includes(".incomplete."));
    assert.equal(quarantined.length, 1, `quarantine: ${quarantined}`);
    assert.equal(currentTarget(context), target);
  });

  it("调用者环境变量覆盖 deploy.env 并回写文件", () => {
    const context = fixture({ targetMigrations: ["0001_registry.sql"], withTargetArtifact: false });
    writeDeployEnv(context.instanceRoot, runtimeVariables);
    const shellOrigin = "https://from-shell.example.test:15402";
    context.env.KNOTED_OFFICE_ORIGIN = shellOrigin;

    const result = deploy(context);
    assert.equal(result.status, 0, result.stderr);
    const buildCall = pcArgs(context.logFile).find((line) => line.startsWith("build "));
    assert.equal(buildCall, `build ${targetRevision} ${join(context.runtimeParentDir, targetRevision)} ${shellOrigin}`);
    assert.match(readFileSync(join(context.instanceRoot, "deploy.env"), "utf8"), new RegExp(`^KNOTED_OFFICE_ORIGIN=${shellOrigin}$`, "m"));
  });

  it("实例停着时 restart 失败回退到完整启动", () => {
    const context = fixture({ targetMigrations: ["0001_registry.sql"] });
    writeDeployEnv(context.instanceRoot, runtimeVariables);
    context.env.STUB_RESTART_FAIL = "1";

    const result = deploy(context);
    assert.equal(result.status, 0, result.stderr);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, targetRevision));
    assert.deepEqual(pcArgs(context.logFile), [
      "pc process restart knoted-api",
      "pc down",
      "pc up -D",
    ]);
  });

  it("首次部署从活动进程并集抓取运行变量并写成 0600", () => {
    const context = fixture({ targetMigrations: ["0001_registry.sql"] });
    // 真实布局：service.sh 用 env -i 只给 intelligence 留模型根，其余开关只在 api/worker。
    const writeEnviron = (pid, values) => {
      writeFileSync(
        join(context.procRoot, pid, "environ"),
        Object.entries(values)
          .map(([key, value]) => `${key}=${value}`)
          .join("\0") + "\0",
      );
    };
    writeEnviron("1002", {
      KNOTED_INTELLIGENCE_DOCLING_ARTIFACTS_DIR: runtimeVariables.KNOTED_INTELLIGENCE_DOCLING_ARTIFACTS_DIR,
      KNOTED_INTELLIGENCE_BGE_M3_DIR: runtimeVariables.KNOTED_INTELLIGENCE_BGE_M3_DIR,
    });
    writeEnviron("1000", {
      KNOTED_DEV_CHAT_CONFIG_DIGEST: "none",
      KNOTED_SEARCH_ENABLED: runtimeVariables.KNOTED_SEARCH_ENABLED,
      KNOTED_VECTORS_ENABLED: runtimeVariables.KNOTED_VECTORS_ENABLED,
      KNOTED_OFFICE_ORIGIN: runtimeVariables.KNOTED_OFFICE_ORIGIN,
      KNOTED_OFFICE_JWT_SECRET_FILE: runtimeVariables.KNOTED_OFFICE_JWT_SECRET_FILE,
    });

    const result = deploy(context);
    assert.equal(result.status, 0, result.stderr);
    const deployEnv = join(context.instanceRoot, "deploy.env");
    assert.equal(statSync(deployEnv).mode & 0o777, 0o600);
    for (const [key, value] of Object.entries(runtimeVariables)) {
      assert.match(readFileSync(deployEnv, "utf8"), new RegExp(`^${key}=${value}$`, "m"), key);
    }
  });

  it("既没有 deploy.env 也抓不到活动进程时在切换前退出", () => {
    const context = fixture({ targetMigrations: ["0001_registry.sql"] });
    rmSync(join(context.procRoot, "1002"), { recursive: true, force: true });

    const result = deploy(context);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /KNOTED_INTELLIGENCE_DOCLING_ARTIFACTS_DIR/);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, previousRevision));
    assert.deepEqual(
      pcArgs(context.logFile).filter((line) => line.startsWith("pc ")),
      [],
    );
  });

  it("拒绝在 .worktrees 里部署稳定实例", () => {
    const sandbox = join(makeTempDir("guard"), ".worktrees", "task");
    mkdirSync(sandbox, { recursive: true });
    const context = fixture({ targetMigrations: ["0001_registry.sql"], runtimeParent: join(sandbox, "runtime") });

    const result = deploy(context);
    assert.equal(result.status, 2);
    assert.match(result.stderr, /worktrees/);
  });

  it("拒绝非完整 SHA 参数", () => {
    const context = fixture({ targetMigrations: ["0001_registry.sql"] });
    const result = deploy(context, ["8b0457e"]);
    assert.equal(result.status, 2);
    assert.match(result.stderr, /40/);
  });

  it("拒绝不存在的提交", () => {
    const context = fixture({ targetMigrations: ["0001_registry.sql"] });
    const result = deploy(context, ["f".repeat(40)]);
    assert.equal(result.status, 2);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, previousRevision));
  });
});
