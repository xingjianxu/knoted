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
  renameSync,
  rmSync,
  statSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { createServer } from "node:net";
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

// build-artifact.sh 的产物形状：只提供部署脚本实际校验的条目。
function createArtifact(parent, revision, migrations = ["0001_registry.sql"]) {
  const root = join(parent, revision);
  mkdirSync(join(root, "bin"), { recursive: true });
  mkdirSync(join(root, "share/knoted/web"), { recursive: true });
  mkdirSync(join(root, "share/knoted/migrations"), { recursive: true });
  mkdirSync(join(root, "share/knoted/intelligence/python/bin"), { recursive: true });
  for (const name of ["knoted-api", "knoted-worker", "knoted-intelligence"]) {
    writeExecutable(join(root, `bin/${name}`), "#!/usr/bin/env bash\nexit 0\n");
  }
  writeExecutable(
    join(root, "share/knoted/intelligence/python/bin/python3.12"),
    '#!/usr/bin/env bash\nexec python3 "$@"\n',
  );
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

// 真实的 /proc/<pid> 同时有世界可读的 cmdline/status 与受 ptrace 校验的 exe。
// service.sh 在 exec 前把运行根解析成具体产物目录，所以 cmdline 的 argv[0] 里带 <sha>。
function createProcFixture(procRoot, artifactRoot) {
  Object.values(processImages).forEach((image, index) => {
    const pidDir = join(procRoot, String(1000 + index));
    mkdirSync(pidDir, { recursive: true });
    symlinkSync(join(artifactRoot, image), join(pidDir, "exe"));
    writeFileSync(join(pidDir, "cmdline"), `${join(artifactRoot, image)}\0`);
    writeFileSync(
      join(pidDir, "status"),
      `Name:\tknoted\nUid:\t${process.getuid()}\t${process.getuid()}\t${process.getuid()}\t${process.getuid()}\n` +
        `Gid:\t${process.getgid()}\t${process.getgid()}\t${process.getgid()}\t${process.getgid()}\n`,
    );
  });
  // 生产实例跑在 Nix store 里，必须不被 dev 部署的进程校验误伤。
  mkdirSync(join(procRoot, "2000"), { recursive: true });
  symlinkSync("/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-knoted/bin/knoted-api", join(procRoot, "2000", "exe"));
  writeFileSync(
    join(procRoot, "2000", "cmdline"),
    "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-knoted/bin/knoted-api\0--config\0/run/knoted-prod-config/app.toml\0",
  );
}

// 内核解析 /proc/<pid>/exe 要求调用者与目标 uid+gid 一致（PTRACE_MODE_READ_FSCREDS）。
// 删掉 exe 模拟跨组：只有 cmdline 可用。
function removeProcImages(procRoot) {
  for (const entry of readdirSync(procRoot)) {
    rmSync(join(procRoot, entry, "exe"), { force: true });
  }
}

// 让假进程的主组与测试运行者不同，触发 gid 诊断。
function setProcGid(procRoot, gid) {
  for (const entry of readdirSync(procRoot)) {
    const statusFile = join(procRoot, entry, "status");
    if (!existsSync(statusFile)) continue;
    writeFileSync(statusFile, `Name:\tknoted\nUid:\t${process.getuid()}\t${process.getuid()}\t${process.getuid()}\t${process.getuid()}\nGid:\t${gid}\t${gid}\t${gid}\t${gid}\n`);
  }
}

// 未解析运行根的布局：cmdline 只剩 <runtime_parent>/current/<image>，判定不到具体产物。
// 只改这实例运行根下的进程；/nix/store 里的生产实例同理要以 bin/knoted-api 结尾，不能误伤。
function unresolveProcCmdlines(procRoot, runtimeParentDir) {
  for (const entry of readdirSync(procRoot)) {
    const cmdlineFile = join(procRoot, entry, "cmdline");
    if (!existsSync(cmdlineFile)) continue;
    const argv0 = readFileSync(cmdlineFile, "utf8").replace(/\0.*$/s, "");
    if (!argv0.startsWith(`${runtimeParentDir}/`)) continue;
    const image = Object.values(processImages).find((candidate) => argv0.endsWith(candidate));
    if (!image) continue;
    writeFileSync(cmdlineFile, `${join(runtimeParentDir, "current", image)}\0`);
  }
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
  local target index pid image argv0
  [ "\${STUB_NO_RELINK:-0}" = 1 ] && return 0
  target=$(readlink -f "\${STUB_RUNTIME_PARENT:?}/current")
  index=0
  for image in bin/knoted-api bin/knoted-worker share/knoted/intelligence/python/bin/python3.12; do
    pid=$((1000 + index))
    # STUB_EXE=0 模拟跨组：/proc/<pid>/exe 读不到，只剩 cmdline。
    if [ "\${STUB_EXE:-1}" = 1 ]; then
      rm -f "\${STUB_PROC:?}/$pid/exe"
      ln -s "$target/$image" "\${STUB_PROC:?}/$pid/exe"
    fi
    # STUB_CMD_ALIAS 模拟未解析运行根的 service.sh：cmdline 使用别名。
    if [ -n "\${STUB_CMD_ALIAS:-}" ]; then
      argv0="\${STUB_RUNTIME_PARENT:?}/$STUB_CMD_ALIAS/$image"
    else
      argv0="$target/$image"
    fi
    printf '%s\\0' "$argv0" >"\${STUB_PROC:?}/$pid/cmdline"
    index=$((index + 1))
  done
}
case "$*" in
"pc process list") exit 0 ;;
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
    KNOTED_BUILD_ROOT: join(base, "build"),
    // 运行变量必须由 deploy.env 或活动进程提供，不能从测试宿主环境串味。
    KNOTED_INTELLIGENCE_DOCLING_ARTIFACTS_DIR: "",
    KNOTED_INTELLIGENCE_BGE_M3_DIR: "",
    KNOTED_SEARCH_ENABLED: "",
    KNOTED_VECTORS_ENABLED: "",
    KNOTED_OFFICE_ORIGIN: "",
    KNOTED_OFFICE_JWT_SECRET_FILE: "",
  };
  return { base, instanceRoot, runtimeParentDir, procRoot, logFile, env, templateRoot };
}

function deploy(context, args = [targetRevision]) {
  return run("bash", [context.script ?? deployScript, ...args], { cwd: repoRoot, env: context.env });
}

// 让部署脚本通过实际 Git common-dir 找到隔离的主检出，既可执行真实 launcher，
// 也能模拟任务 worktree 已更新、主检出的 launcher 仍旧的情况。
function attachLauncher(context, { legacy = false } = {}) {
  const checkout = join(context.base, "main-checkout");
  const linked = join(context.base, "linked-checkout");
  const clone = run("git", ["clone", "--quiet", "--shared", "--no-checkout", repoRoot, checkout]);
  assert.equal(clone.status, 0, clone.stderr);
  const worktree = run("git", ["-C", checkout, "worktree", "add", "--quiet", "--detach", linked, targetRevision]);
  assert.equal(worktree.status, 0, worktree.stderr);
  cpSync(join(repoRoot, "process-compose.yaml"), join(checkout, "process-compose.yaml"));
  const launcher = join(checkout, "tools/dev/pc/service.sh");
  mkdirSync(resolve(launcher, ".."), { recursive: true });
  if (legacy) {
    writeExecutable(launcher, '#!/usr/bin/env bash\nprintf "runtime_root=%s\\n" "$KNOTED_RUNTIME_ROOT"\n');
  } else {
    cpSync(join(repoRoot, "tools/dev/pc/service.sh"), launcher);
  }
  context.script = join(linked, "tools/dev/deploy-dev.sh");
  cpSync(deployScript, context.script);
  return checkout;
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
    rmSync(join(context.runtimeParentDir, previousRevision), { recursive: true, force: true });
    rmSync(join(context.runtimeParentDir, "current"));
    symlinkSync(join(context.runtimeParentDir, targetRevision), join(context.runtimeParentDir, "current"));
    rmSync(context.procRoot, { recursive: true, force: true });
    mkdirSync(context.procRoot, { recursive: true });
    createProcFixture(context.procRoot, join(context.runtimeParentDir, targetRevision));

    const result = deploy(context);
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(pcArgs(context.logFile), []);
    assert.match(result.stderr, /已是最新/);
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
    // 重启“成功”，argv 已指向目标，但可读的 exe 仍表明映像没换。
    context.env.STUB_EXE = "0";

    const result = deploy(context);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /仍运行在/);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, previousRevision));
  });

  it("exe 不可读时用 cmdline 判定进程映像并部署成功", () => {
    const context = fixture({ targetMigrations: ["0001_registry.sql"] });
    writeDeployEnv(context.instanceRoot, runtimeVariables);
    // 跨组：/proc/<pid>/exe 一律 EPERM，只剩世界可读的 cmdline。
    removeProcImages(context.procRoot);
    context.env.STUB_EXE = "0";

    const result = deploy(context);
    assert.equal(result.status, 0, result.stderr);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, targetRevision));
    assert.match(result.stderr, /cmdline/);
    assert.doesNotMatch(result.stderr, /没有找到运行中的 knoted-api/);
  });

  it("从历史短 SHA 目录升级时由主检出的 launcher 固定新产物路径", () => {
    const context = fixture({ targetMigrations: ["0001_registry.sql"] });
    const shortRoot = join(context.runtimeParentDir, "4a546b2f0efd");
    renameSync(currentTarget(context), shortRoot);
    const current = join(context.runtimeParentDir, "current");
    rmSync(current);
    symlinkSync(shortRoot, current);
    writeDeployEnv(context.instanceRoot, runtimeVariables);
    // 旧进程由未解析运行根的 service.sh 启动：exe 不可读，cmdline 只有 current。
    // 重启后会由本次的 service.sh 重新拉起，所以这不应该阻止部署。
    removeProcImages(context.procRoot);
    unresolveProcCmdlines(context.procRoot, context.runtimeParentDir);
    setProcGid(context.procRoot, 980);
    const checkout = attachLauncher(context);

    const result = deploy(context);

    assert.equal(result.status, 0, result.stderr);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, targetRevision));
    assert.deepEqual(pcCwd(context.logFile), [checkout]);
    assert.match(result.stderr, /主检出的 launcher 能为新进程固定产物路径/);
  });

  it("主检出仍用旧 launcher 时不切换产物也不重启", () => {
    const context = fixture();
    writeDeployEnv(context.instanceRoot, runtimeVariables);
    removeProcImages(context.procRoot);
    unresolveProcCmdlines(context.procRoot, context.runtimeParentDir);
    setProcGid(context.procRoot, 980);
    attachLauncher(context, { legacy: true });

    const result = deploy(context);
    assert.equal(result.status, 2, result.stderr);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, previousRevision));
    assert.deepEqual(pcArgs(context.logFile), []);
    assert.match(result.stderr, /先更新主检出.*sg .*未切换指针/);
  });

  it("有可读但不匹配的 exe 时不使用指向目标的 argv", () => {
    const context = fixture();
    writeDeployEnv(context.instanceRoot, runtimeVariables);
    removeProcImages(context.procRoot);
    symlinkSync("/usr/bin/bash", join(context.procRoot, "1000", "exe"));
    context.env.STUB_EXE = "0";

    const result = deploy(context);
    assert.equal(result.status, 1, result.stderr);
    assert.match(result.stderr, /没有找到运行中的 knoted-api/);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, previousRevision));
  });

  it("仅有 cmdline 但指向旧 SHA 时仍复位", () => {
    const context = fixture();
    writeDeployEnv(context.instanceRoot, runtimeVariables);
    removeProcImages(context.procRoot);
    context.env.STUB_NO_RELINK = "1";

    const result = deploy(context);
    assert.equal(result.status, 1, result.stderr);
    assert.match(result.stderr, /仍运行在/);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, previousRevision));
  });

  it("切换后无法判定进程映像时立即失败且不回滚", () => {
    const context = fixture({ targetMigrations: ["0001_registry.sql"] });
    writeDeployEnv(context.instanceRoot, runtimeVariables);
    removeProcImages(context.procRoot);
    // 切换前 cmdline 有具体产物目录；重启后仅引用自定义别名，不能当成旧 SHA。
    context.env.STUB_EXE = "0";
    context.env.STUB_CMD_ALIAS = "configured-alias";
    symlinkSync(join(context.runtimeParentDir, "current"), join(context.runtimeParentDir, "configured-alias"));
    context.env.KNOTED_DEPLOY_READY_TIMEOUT = "60";

    const started = Date.now();
    const result = deploy(context);
    const elapsed = Date.now() - started;

    assert.notEqual(result.status, 0);
    // 判不出来时回滚只会扩大空窗，所以指针留在新产物，也不重复重启。
    assert.equal(currentTarget(context), join(context.runtimeParentDir, targetRevision));
    assert.deepEqual(pcArgs(context.logFile), [
      "pc process restart knoted-api",
      "pc process restart knoted-worker",
      "pc process restart knoted-intelligence",
    ]);
    assert.ok(elapsed < 20_000, `必须快速失败，实际 ${elapsed}ms`);
    assert.match(result.stderr, /不回滚/);
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

  it("只有 cmdline 的候选进程不能提供部署环境变量", () => {
    const context = fixture();
    removeProcImages(context.procRoot);
    const input = Object.entries(runtimeVariables).map(([key, value]) => `${key}=${value}\0`).join("");
    writeFileSync(join(context.procRoot, "1000", "environ"), input);

    const result = deploy(context);
    assert.equal(result.status, 2, result.stderr);
    assert.match(result.stderr, /environ 不可读或无法确认来源/);
    assert.equal(existsSync(join(context.instanceRoot, "deploy.env")), false);
    assert.equal(currentTarget(context), join(context.runtimeParentDir, previousRevision));
    assert.deepEqual(pcArgs(context.logFile), []);
  });

  it("environ 存在且有读权限但实际打不开时给出诊断并在切换前退出", async () => {
    const context = fixture();
    // 用 Unix socket 模拟 stat/-r 通过、实际打开失败（不依赖测试用户权限）。
    const environ = join(context.procRoot, "1002", "environ");
    const server = createServer();
    await new Promise((resolve, reject) => server.once("error", reject).listen(environ, resolve));
    let result;
    try {
      chmodSync(environ, 0o600);
      assert.equal(run("test", ["-r", environ]).status, 0);
      result = deploy(context);
    } finally {
      await new Promise((resolve) => server.close(resolve));
    }
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /KNOTED_INTELLIGENCE_DOCLING_ARTIFACTS_DIR/);
    // 有活动进程但 environ 读不到（如跨组）时不能只说“缺少变量”。
    assert.match(result.stderr, /environ 不可读/);
    assert.match(result.stderr, /sg /);
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
