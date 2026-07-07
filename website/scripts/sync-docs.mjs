#!/usr/bin/env node
// Syncs ../docs/*.md -> src/pages/docs/*.md for the Astro site.
//
// docs/ is the single source of truth. This strips the leading "# Title" line,
// rewrites relative .md links to website URLs, and prepends Astro frontmatter
// (layout + title + description). Generated files are .gitignored; never edit
// src/pages/docs/ by hand.

import { readFileSync, writeFileSync, mkdirSync } from "fs";
import { dirname, join } from "path";
import { fileURLToPath } from "url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const ROOT = join(__dirname, "..", ".."); // repo root
const PAGES_DIR = join(__dirname, "..", "src", "pages");

// source is relative to repo root; dest relative to src/pages/.
const PAGES = [
  { source: "docs/index.md", dest: "docs/index.md", title: "Overview",
    description: "CityHall: the self-hostable control-plane server for Agent of Empires." },
  { source: "docs/quick-start.md", dest: "docs/quick-start.md", title: "Quick Start",
    description: "Run CityHall, sign in, and manage users." },
  { source: "docs/configuration.md", dest: "docs/configuration.md", title: "Configuration",
    description: "Database, bind address, logging, email/SMTP, SSO, and self-signup." },
  { source: "docs/deployment.md", dest: "docs/deployment.md", title: "Deployment",
    description: "Docker, Compose, Kubernetes, Helm, VPS, HTTPS/reverse proxy, and databases." },
  { source: "docs/cli.md", dest: "docs/cli.md", title: "CLI Reference",
    description: "Manage users and run the server from the command line." },
  { source: "docs/api.md", dest: "docs/api.md", title: "API Reference",
    description: "The HTTP endpoints CityHall serves under /api." },
  { source: "docs/development.md", dest: "docs/development.md", title: "Development",
    description: "Build, test, and extend CityHall." },
];

// docs/*.md path -> website URL, for link rewriting.
const URL_MAP = {
  "docs/index.md": "/docs/",
  "docs/quick-start.md": "/docs/quick-start/",
  "docs/configuration.md": "/docs/configuration/",
  "docs/deployment.md": "/docs/deployment/",
  "docs/cli.md": "/docs/cli/",
  "docs/api.md": "/docs/api/",
  "docs/development.md": "/docs/development/",
};

const GITHUB_BASE = "https://github.com/agent-of-empires/cityhall/blob/main/";

function rewriteLinks(content, sourceDir) {
  return content.replace(/\]\(([^)]+\.md(?:#[^)]*)?)\)/g, (_m, link) => {
    if (link.startsWith("http")) return `](${link})`;
    const hashIdx = link.indexOf("#");
    const target = hashIdx >= 0 ? link.slice(0, hashIdx) : link;
    const anchor = hashIdx >= 0 ? link.slice(hashIdx) : "";
    const resolved = join(sourceDir, target).replace(/\\/g, "/").replace(/^\.\//, "");
    // ../deploy/... and other non-docs paths fall through to GitHub.
    const url = URL_MAP[resolved];
    return url ? `](${url}${anchor})` : `](${GITHUB_BASE}${resolved}${anchor})`;
  });
}

function computeLayoutPath(dest) {
  const segments = dirname(dest).split("/").filter((s) => s !== ".");
  const depth = segments.length + 1; // pages/ -> src/
  return "../".repeat(depth) + "layouts/Docs.astro";
}

function escapeYaml(str) {
  return /[:"'\\]/.test(str) ? `"${str.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"` : str;
}

console.log("Syncing docs to website...");
for (const page of PAGES) {
  let content = readFileSync(join(ROOT, page.source), "utf8");
  content = content.replace(/^# .+\n\n?/, "");
  content = rewriteLinks(content, dirname(page.source));
  const frontmatter = [
    "---",
    `layout: ${computeLayoutPath(page.dest)}`,
    `title: ${escapeYaml(page.title)}`,
    `description: ${escapeYaml(page.description)}`,
    "---",
    "",
    "",
  ].join("\n");
  const destPath = join(PAGES_DIR, page.dest);
  mkdirSync(dirname(destPath), { recursive: true });
  writeFileSync(destPath, frontmatter + content);
  console.log(`  ${page.source} -> ${page.dest}`);
}

// Fail if a sidebar entry has no synced page (nav and pages must not drift).
const navSource = readFileSync(join(__dirname, "..", "src", "data", "docsNav.ts"), "utf8");
const navHrefs = new Set([...navSource.matchAll(/href:\s*"([^"]+)"/g)].map((m) => m[1]));
const synced = new Set(PAGES.map((p) => "/" + p.dest.replace(/\.md$/, "/").replace(/\/index\/$/, "/")));
let missing = 0;
for (const href of navHrefs) {
  if (!synced.has(href)) {
    console.error(`  WARNING: sidebar href ${href} has no synced page`);
    missing++;
  }
}
if (missing > 0) {
  console.error(`\n${missing} sidebar link(s) without a page (fix src/data/docsNav.ts or PAGES)`);
  process.exit(1);
}
console.log("Done.");
