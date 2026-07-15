import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import test from "node:test";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

test("renders a public npm-backed Homebrew formula", () => {
  const work = fs.mkdtempSync(path.join(os.tmpdir(), "ycloud-homebrew-"));
  const version = "9.8.7";
  const platforms = [
    "darwin-arm64",
    "darwin-x64",
    "linux-arm64",
    "linux-x64",
  ];

  for (const platform of platforms) {
    fs.writeFileSync(
      path.join(
        work,
        `ycloud-ai-console-cli-${platform}-${version}.tgz`,
      ),
      platform,
    );
  }

  const output = path.join(work, "ycloud.rb");
  const result = spawnSync(
    process.execPath,
    [
      path.join(root, "scripts/render-homebrew-formula.mjs"),
      version,
      work,
      output,
    ],
    { encoding: "utf8" },
  );
  assert.equal(result.status, 0, result.stderr);

  const formula = fs.readFileSync(output, "utf8");
  assert.match(
    formula,
    /https:\/\/registry\.npmjs\.org\/@ycloud-ai\/console-cli-darwin-arm64\/-\/console-cli-darwin-arm64-9\.8\.7\.tgz/,
  );
  assert.match(formula, /bin\.install "package\/bin\/ycloud"/);
  assert.doesNotMatch(formula, /github\.com.*releases\/download/);
});
