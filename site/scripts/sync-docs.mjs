// docs/ is the single source of truth; this copies it into Starlight's
// content directory, deriving each page's title from its first heading.
import { readFileSync, writeFileSync, mkdirSync, readdirSync, rmSync, cpSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const base = process.env.DOCS_BASE ?? "/exacttex/";
const prefix = base.endsWith("/") ? base : base + "/";
const repo = join(here, "..", "..");
const out = join(here, "..", "src", "content", "docs");

rmSync(out, { recursive: true, force: true });
mkdirSync(join(out, "decisions"), { recursive: true });

function page(src, dest, titleOverride, order) {
  let text = readFileSync(src, "utf8");
  const heading = text.match(/^#\s+(.+)$/m);
  const title = titleOverride ?? (heading ? heading[1].replace(/[`*]/g, "") : dest);
  if (heading) text = text.replace(/^#\s+.+\n/m, "");
  // Root-relative repo links have no meaning on the site; point them at GitHub.
  text = text.replaceAll("](docs/", `](${prefix}`);
  text = text.replaceAll('src="docs/assets/', `src="${prefix}assets/`);
  const front = ["---", `title: "${title.replaceAll('"', '\\"')}"`];
  if (order !== undefined) front.push(`sidebar:`, `  order: ${order}`);
  front.push("---", "");
  writeFileSync(join(out, dest), front.join("\n") + text);
}

writeFileSync(join(out, "index.mdx"), `---
title: ExactTeX
description: Know whether your document is sound before you look at the PDF.
template: splash
hero:
  title: Know whether your document is sound <em>before you look at the PDF</em>.
  tagline: A document language with LaTeX as its backend. Rename your .tex and it keeps working — what you annotate is guaranteed by a compiler.
  image:
    light: ../../assets/exacttex-logo.svg
    dark: ../../assets/exacttex-logo-dark.svg
  actions:
    - text: Introduction
      link: ${prefix}introduction/
      icon: right-arrow
    - text: The grammar
      link: ${prefix}grammar/
      variant: minimal
    - text: GitHub
      link: https://github.com/camilochs/exacttex
      icon: github
      variant: minimal
---

import { Card, CardGrid } from '@astrojs/starlight/components';

<CardGrid stagger>
  <Card title="Gradual, like TypeScript" icon="approve-check">
    Every valid .tex file is a valid ExactTeX document. Annotate one
    identifier and exactly that pair is checked; the coverage number tells
    you how much of the document is under contract.
  </Card>
  <Card title="Errors in your own words" icon="magnifier">
    A closed table of fourteen hard errors, each at the site where the fix
    belongs — with the declaration it points at, one click away.
  </Card>
  <Card title="Revisions inside the file" icon="pencil">
    @add, @del, @sub and @note live in the document. Any tool reads the same
    anchored changes; an agent suggests, a human accepts.
  </Card>
  <Card title="Three doors, one answer" icon="puzzle">
    CLI, language server and WebAssembly build share one core, and a parity
    suite holds them byte-for-byte to the same output.
  </Card>
</CardGrid>
`);
// The README's centered logo-and-badges header belongs to GitHub; the site
// already wears the logo, and a private repo's CI badge 404s anonymously.
// Introduction starts at the first sentence of prose.
{
  const raw = readFileSync(join(repo, "README.md"), "utf8");
  const start = raw.indexOf("ExactTeX is a document language");
  const trimmed = start > 0 ? raw.slice(start) : raw;
  const tmp = join(here, "introduction.tmp.md");
  writeFileSync(tmp, trimmed);
  page(tmp, "introduction.md", "Introduction");
  rmSync(tmp);
}
page(join(repo, "PHILOSOPHY.md"), "philosophy.md", "Philosophy");
page(join(repo, "ROADMAP.md"), "roadmap.md", "Roadmap");
page(join(repo, "CONTRIBUTING.md"), "contributing.md", "Contributing");
for (const name of ["grammar", "checking", "revisions", "diagnostics", "wasm", "lsp", "testing", "references"]) {
  page(join(repo, "docs", `${name}.md`), `${name}.md`);
}
let order = 1;
for (const file of readdirSync(join(repo, "docs", "decisions")).sort()) {
  if (!file.endsWith(".md")) continue;
  page(join(repo, "docs", "decisions", file), join("decisions", file), undefined, order++);
}
cpSync(join(repo, "docs", "assets"), join(here, "..", "public", "assets"), { recursive: true });
console.log("docs synced");
