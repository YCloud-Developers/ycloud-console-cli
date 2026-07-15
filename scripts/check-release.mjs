import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const cargo = fs.readFileSync(path.join(root, "Cargo.toml"), "utf8");
const cargoVersion = cargo.match(/^version = "([^"]+)"$/m)?.[1];
if (!cargoVersion) {
  throw new Error("Cargo.toml package version is missing");
}

const packageFiles = [
  "package.json",
  "npm/console-cli/package.json",
  "npm/platforms/darwin-arm64/package.json",
  "npm/platforms/darwin-x64/package.json",
  "npm/platforms/linux-arm64/package.json",
  "npm/platforms/linux-x64/package.json",
];

for (const relativePath of packageFiles) {
  const packageJson = JSON.parse(
    fs.readFileSync(path.join(root, relativePath), "utf8"),
  );
  if (packageJson.version !== cargoVersion) {
    throw new Error(
      `${relativePath} version ${packageJson.version} does not match Cargo.toml ${cargoVersion}`,
    );
  }
  if (packageJson.license !== "MIT") {
    throw new Error(`${relativePath} must declare the MIT license`);
  }

  const packageDirectory = path.dirname(path.join(root, relativePath));
  if (!fs.existsSync(path.join(packageDirectory, "LICENSE"))) {
    throw new Error(`${relativePath} package is missing LICENSE`);
  }
}

const launcher = JSON.parse(
  fs.readFileSync(path.join(root, "npm/console-cli/package.json"), "utf8"),
);
for (const [packageName, version] of Object.entries(
  launcher.optionalDependencies,
)) {
  if (version !== cargoVersion) {
    throw new Error(
      `${packageName} dependency ${version} does not match Cargo.toml ${cargoVersion}`,
    );
  }
}

const tag = process.env.GITHUB_REF_NAME;
if (tag && tag !== `v${cargoVersion}`) {
  throw new Error(`tag ${tag} does not match version v${cargoVersion}`);
}

console.log(`Release metadata is consistent for v${cargoVersion}.`);
