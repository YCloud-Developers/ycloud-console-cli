import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
export const packages = {
  "darwin-arm64": { packageName: "darwin-arm64", executable: "ycloud" },
  "darwin-x64": { packageName: "darwin-x64", executable: "ycloud" },
  "linux-arm64": { packageName: "linux-arm64", executable: "ycloud" },
  "linux-x64": { packageName: "linux-x64", executable: "ycloud" },
  "windows-arm64": { packageName: "windows-arm64", executable: "ycloud.exe" },
  "windows-x64": { packageName: "windows-x64", executable: "ycloud.exe" },
};

export function stageNpmPackages(
  artifacts,
  platformsRoot = path.join(root, "npm", "platforms"),
) {
  for (const [artifactName, { packageName, executable }] of Object.entries(
    packages,
  )) {
    const source = path.join(
      artifacts,
      `release-${artifactName}`,
      executable,
    );
    const destination = path.join(
      platformsRoot,
      packageName,
      "bin",
      executable,
    );
    if (!fs.existsSync(source)) {
      throw new Error(`release binary is missing: ${source}`);
    }
    fs.mkdirSync(path.dirname(destination), { recursive: true });
    fs.copyFileSync(source, destination);
    if (process.platform !== "win32") {
      fs.chmodSync(destination, 0o755);
    }
  }
}

if (path.resolve(process.argv[1] ?? "") === fileURLToPath(import.meta.url)) {
  if (!process.argv[2]) {
    throw new Error("usage: node scripts/stage-npm-packages.mjs <artifacts-dir>");
  }
  stageNpmPackages(path.resolve(process.argv[2]));
  console.log("Native npm packages staged.");
}
