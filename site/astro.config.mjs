// The documentation site. `docs/` in the repository root stays the single
// source of truth; `scripts/sync-docs.mjs` copies it here with frontmatter
// at build time, so nothing is written twice.
import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";

export default defineConfig({
  site: "https://camilochs.github.io",
  // The Pages deploy lives under /exacttex; a tailnet preview builds with
  // DOCS_BASE=/ so it can be served from any static server's root.
  base: process.env.DOCS_BASE ?? "/exacttex",
  integrations: [
    starlight({
      title: "ExactTeX",
      logo: { src: "./src/assets/exacttex-logo.svg", replacesTitle: true },
      customCss: ["./src/styles/theme.css"],
      social: [{ icon: "github", label: "GitHub", href: "https://github.com/camilochs/exacttex" }],
      sidebar: [
        { label: "Start", items: [
          { label: "Introduction", slug: "introduction" },
          { label: "Philosophy", slug: "philosophy" },
        ]},
        { label: "The language", items: [
          { label: "Grammar", slug: "grammar" },
          { label: "Checking", slug: "checking" },
          { label: "Revisions", slug: "revisions" },
        ]},
        { label: "The three doors", items: [
          { label: "Diagnostics & blame", slug: "diagnostics" },
          { label: "WebAssembly", slug: "wasm" },
          { label: "Language server", slug: "lsp" },
        ]},
        { label: "The project", items: [
          { label: "Roadmap", slug: "roadmap" },
          { label: "Testing", slug: "testing" },
          { label: "References", slug: "references" },
          { label: "Contributing", slug: "contributing" },
        ]},
        { label: "Decisions", autogenerate: { directory: "decisions" } },
      ],
    }),
  ],
});
