#!/usr/bin/env node

// Generates a winget manifest YAML for HooCode releases.

import { writeFileSync } from "fs";
// Usage:
//   node scripts/generate-winget-manifest.mjs <version> <sha256> <release-url> [--output <filepath>]

const USAGE = `Usage: node scripts/generate-winget-manifest.mjs <version> <sha256> <release-url> [--output <filepath>]

Arguments:
  version      Package version (e.g., 0.2.0)
  sha256       SHA256 hash of the installer binary (hex string)
  release-url  Direct download URL for the portable .exe

Options:
  --output <filepath>  Write manifest to file instead of stdout
  --help               Show this help message

Example:
  node scripts/generate-winget-manifest.mjs 0.2.0 abcdef... https://github.com/.../hoocode-windows-x64.exe
`;

const HEX_RE = /^[0-9a-fA-F]+$/;
const URL_RE = /^https?:\/\/.+/;

function validate(args) {
  const errors = [];
  if (!args.version) errors.push("version is required");
  if (!args.sha256) errors.push("sha256 is required");
  else if (!HEX_RE.test(args.sha256))
    errors.push("sha256 must be a hex string");
  else if (args.sha256.length !== 64)
    errors.push("sha256 must be exactly 64 hex characters");
  if (!args.url) errors.push("release-url is required");
  else if (!URL_RE.test(args.url))
    errors.push("release-url must be a valid URL (http/https)");
  return errors;
}

function generateYaml(version, sha256, url) {
  return [
    "PackageIdentifier: kolisachint.hoocode",
    `PackageVersion: ${version}`,
    "PackageLocale: en-US",
    "Publisher: Sachin Koli",
    "PublisherUrl: https://github.com/kolisachint/hoocode",
    "PublisherSupportUrl: https://github.com/kolisachint/hoocode/issues",
    "PackageName: HooCode",
    "License: MIT",
    "LicenseUrl: https://github.com/kolisachint/hoocode/blob/main/LICENSE",
    "ShortDescription: Deterministic terminal coding agent with profile-aware customization",
    "Installers:",
    "  - Architecture: x64",
    "    InstallerType: portable",
    `    InstallerUrl: ${url}`,
    `    InstallerSha256: ${sha256}`,
    "ManifestType: singleton",
    "ManifestVersion: 1.6.0",
    "",
  ].join("\n");
}

function parseArgs(argv) {
  const args = { version: null, sha256: null, url: null, output: null };
  const positional = [];
  let i = 2;

  while (i < argv.length) {
    const arg = argv[i];
    if (arg === "--help") {
      return { ...args, help: true };
    } else if (arg === "--output") {
      i++;
      if (i >= argv.length) {
        console.error("error: --output requires a file path argument");
        process.exit(1);
      }
      args.output = argv[i];
    } else {
      positional.push(arg);
    }
    i++;
  }

  if (positional.length > 0) args.version = positional[0];
  if (positional.length > 1) args.sha256 = positional[1];
  if (positional.length > 2) args.url = positional[2];

  return args;
}

function main() {
  const args = parseArgs(process.argv);

  if (args.help || !args.version) {
    process.stdout.write(USAGE);
    process.exit(args.help ? 0 : 1);
  }

  const errors = validate(args);
  if (errors.length > 0) {
    console.error("Validation errors:");
    for (const err of errors) {
      console.error(`  - ${err}`);
    }
    console.error();
    process.stderr.write(USAGE);
    process.exit(1);
  }

  const yaml = generateYaml(args.version, args.sha256, args.url);

  if (args.output) {
    writeFileSync(args.output, yaml, "utf-8");
    console.log(`Manifest written to ${args.output}`);
  } else {
    process.stdout.write(yaml);
  }
}

main();
