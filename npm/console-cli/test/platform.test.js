const assert = require("node:assert/strict");
const test = require("node:test");
const { executableName, platformPackage } = require("../lib/platform");

test("maps supported platforms to scoped native packages", () => {
  assert.equal(
    platformPackage("darwin", "arm64"),
    "@ycloud-ai/console-cli-darwin-arm64",
  );
  assert.equal(
    platformPackage("darwin", "x64"),
    "@ycloud-ai/console-cli-darwin-x64",
  );
  assert.equal(
    platformPackage("linux", "arm64"),
    "@ycloud-ai/console-cli-linux-arm64",
  );
  assert.equal(
    platformPackage("linux", "x64"),
    "@ycloud-ai/console-cli-linux-x64",
  );
});

test("rejects unsupported platforms", () => {
  assert.throws(
    () => platformPackage("freebsd", "x64"),
    /unsupported platform: freebsd\/x64/,
  );
});

test("uses the Windows executable suffix only on Windows", () => {
  assert.equal(executableName("darwin"), "ycloud");
  assert.equal(executableName("linux"), "ycloud");
  assert.equal(executableName("win32"), "ycloud.exe");
});
