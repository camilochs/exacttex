# 0008 · The documentation site is Starlight, and docs/ stays the source

**Status:** accepted, 2026-08-30, preparing the repository for the public.

The site under `site/` renders this repository's markdown with Astro's
Starlight. Two rules keep it honest:

1. **`docs/` and the top-level documents remain the single source of truth.**
   `site/scripts/sync-docs.mjs` copies them into the site at build time,
   deriving titles from first headings. Nothing is written twice; editing a
   page means editing the original file.
2. **The dependency boundary is unchanged.** The compiler's crates keep zero
   dependencies; `site/` is publishing tooling with its own `package.json`,
   never linked from any crate, in the same class as CI's actions.

Starlight over mdBook (the Rust default) for one reason: the result. Built-in
search, dark mode, and a first page worth landing on, all themed to the
language's palette in thirty lines of CSS. Deploys to GitHub Pages via
`.github/workflows/docs.yml` when the repository is public.
