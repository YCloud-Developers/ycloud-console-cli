import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";

const [version, artifactsArg, outputArg] = process.argv.slice(2);
if (!version || !artifactsArg || !outputArg) {
  throw new Error(
    "usage: node scripts/render-homebrew-formula.mjs <version> <artifacts-dir> <output>",
  );
}

const tarballs = path.resolve(artifactsArg);
const output = path.resolve(outputArg);
const packages = {
  "darwin-arm64": "@ycloud-ai/console-cli-darwin-arm64",
  "darwin-x64": "@ycloud-ai/console-cli-darwin-x64",
  "linux-arm64": "@ycloud-ai/console-cli-linux-arm64",
  "linux-x64": "@ycloud-ai/console-cli-linux-x64",
};

function asset(platform) {
  const packageName = packages[platform];
  const unscopedName = packageName.split("/")[1];
  const localName = `${packageName.slice(1).replace("/", "-")}-${version}.tgz`;
  const filePath = path.join(tarballs, localName);
  const checksum = crypto
    .createHash("sha256")
    .update(fs.readFileSync(filePath))
    .digest("hex");
  return {
    checksum,
    url: `https://registry.npmjs.org/${packageName}/-/${unscopedName}-${version}.tgz`,
  };
}

const darwinArm64 = asset("darwin-arm64");
const darwinX64 = asset("darwin-x64");
const linuxArm64 = asset("linux-arm64");
const linuxX64 = asset("linux-x64");

const formula = `# typed: strict
# frozen_string_literal: true

# YCloud Console CLI.
class Ycloud < Formula
  desc "Console-oriented YCloud CLI using browser grant authentication"
  homepage "https://www.ycloud.com"
  version "${version}"

  on_macos do
    on_arm do
      url "${darwinArm64.url}"
      sha256 "${darwinArm64.checksum}"
    end

    on_intel do
      url "${darwinX64.url}"
      sha256 "${darwinX64.checksum}"
    end
  end

  on_linux do
    on_arm do
      url "${linuxArm64.url}"
      sha256 "${linuxArm64.checksum}"
    end

    on_intel do
      url "${linuxX64.url}"
      sha256 "${linuxX64.checksum}"
    end
  end

  def install
    bin.install "package/bin/ycloud"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/ycloud --version")
  end
end
`;

fs.mkdirSync(path.dirname(output), { recursive: true });
fs.writeFileSync(output, formula);
console.log(`Homebrew formula written to ${output}`);
