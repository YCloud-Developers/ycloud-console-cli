import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import test from "node:test";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const script = path.join(root, "scripts/generate-release-checksums.mjs");

function checksum(content) {
  return crypto.createHash("sha256").update(content).digest("hex");
}

test("writes portable checksums using release asset names", () => {
  const work = fs.mkdtempSync(path.join(os.tmpdir(), "ycloud-checksums-"));
  const artifacts = path.join(work, "artifacts");
  fs.mkdirSync(path.join(artifacts, "release-linux"), { recursive: true });
  fs.mkdirSync(path.join(artifacts, "release-darwin"), { recursive: true });
  fs.writeFileSync(path.join(artifacts, "release-linux", "ycloud-linux.tar.gz"), "linux");
  fs.writeFileSync(path.join(artifacts, "release-darwin", "ycloud-darwin.tar.gz"), "darwin");

  const output = path.join(work, "SHA256SUMS");
  const result = spawnSync(process.execPath, [script, artifacts, output], {
    encoding: "utf8",
  });
  assert.equal(result.status, 0, result.stderr);
  assert.equal(
    fs.readFileSync(output, "utf8"),
    `${checksum("darwin")}  ycloud-darwin.tar.gz\n${checksum("linux")}  ycloud-linux.tar.gz\n`,
  );
});

test("rejects duplicate release asset names", () => {
  const work = fs.mkdtempSync(path.join(os.tmpdir(), "ycloud-checksums-"));
  const artifacts = path.join(work, "artifacts");
  fs.mkdirSync(path.join(artifacts, "one"), { recursive: true });
  fs.mkdirSync(path.join(artifacts, "two"), { recursive: true });
  fs.writeFileSync(path.join(artifacts, "one", "ycloud.tar.gz"), "one");
  fs.writeFileSync(path.join(artifacts, "two", "ycloud.tar.gz"), "two");

  const result = spawnSync(
    process.execPath,
    [script, artifacts, path.join(work, "SHA256SUMS")],
    { encoding: "utf8" },
  );
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /duplicate release archive name/);
});
