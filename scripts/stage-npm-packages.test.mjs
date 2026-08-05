import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { packages, stageNpmPackages } from "./stage-npm-packages.mjs";

test("stages every native executable including both Windows architectures", () => {
  const work = fs.mkdtempSync(path.join(os.tmpdir(), "ycloud-npm-stage-"));
  const artifacts = path.join(work, "artifacts");
  const platformsRoot = path.join(work, "platforms");

  for (const [artifactName, { executable }] of Object.entries(packages)) {
    const sourceDirectory = path.join(artifacts, `release-${artifactName}`);
    fs.mkdirSync(sourceDirectory, { recursive: true });
    fs.writeFileSync(
      path.join(sourceDirectory, executable),
      `binary-${artifactName}`,
    );
  }

  stageNpmPackages(artifacts, platformsRoot);

  for (const [artifactName, { packageName, executable }] of Object.entries(
    packages,
  )) {
    assert.equal(
      fs.readFileSync(
        path.join(platformsRoot, packageName, "bin", executable),
        "utf8",
      ),
      `binary-${artifactName}`,
    );
  }

  assert.ok(
    fs.existsSync(
      path.join(platformsRoot, "windows-arm64", "bin", "ycloud.exe"),
    ),
  );
  assert.ok(
    fs.existsSync(
      path.join(platformsRoot, "windows-x64", "bin", "ycloud.exe"),
    ),
  );
});

test("fails when a release artifact is missing", () => {
  const work = fs.mkdtempSync(path.join(os.tmpdir(), "ycloud-npm-stage-"));
  assert.throws(
    () => stageNpmPackages(work, path.join(work, "platforms")),
    /release binary is missing/,
  );
});
