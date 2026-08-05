import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { publishIfMissing } from "./publish-if-missing.mjs";

function createPackage() {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "publish-package-"));
  fs.writeFileSync(
    path.join(directory, "package.json"),
    JSON.stringify({ name: "@example/cli", version: "1.2.3" }),
  );
  return directory;
}

test("skips a package version that already exists", () => {
  const calls = [];
  const messages = [];
  const result = publishIfMissing(createPackage(), {
    runNpm(args) {
      calls.push(args);
      return { status: 0, stdout: '"1.2.3"\n', stderr: "" };
    },
    log(message) {
      messages.push(message);
    },
  });

  assert.equal(result, "skipped");
  assert.deepEqual(calls, [
    ["view", "@example/cli@1.2.3", "version", "--json", "--prefer-online"],
  ]);
  assert.deepEqual(messages, [
    "@example/cli@1.2.3 already exists; skipping publish.",
  ]);
});

test("publishes a package version that does not exist", () => {
  const directory = createPackage();
  const calls = [];
  const result = publishIfMissing(directory, {
    runNpm(args) {
      calls.push(args);
      if (args[0] === "view") {
        return { status: 1, stdout: "", stderr: "npm error code E404" };
      }
      return { status: 0, stdout: "", stderr: "" };
    },
  });

  assert.equal(result, "published");
  assert.deepEqual(calls, [
    ["view", "@example/cli@1.2.3", "version", "--json", "--prefer-online"],
    ["publish", path.resolve(directory), "--access", "public"],
  ]);
});

test("does not publish when the registry lookup fails", () => {
  let callCount = 0;
  assert.throws(
    () =>
      publishIfMissing(createPackage(), {
        runNpm() {
          callCount += 1;
          return {
            status: 1,
            stdout: "",
            stderr: "npm error code E401",
          };
        },
      }),
    /failed to check @example\/cli@1\.2\.3 on npm/,
  );
  assert.equal(callCount, 1);
});
