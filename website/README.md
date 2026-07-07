# CityHall docs website

Astro static site that renders the markdown in [`../docs`](../docs) as a
browsable documentation website (same approach as the Agent of Empires site).

`../docs/*.md` is the single source of truth. `scripts/sync-docs.mjs` copies it
into `src/pages/docs/` (rewriting links, adding frontmatter) on every dev/build;
those generated files are gitignored. Edit the docs, not the generated pages.

## View locally

```sh
cd website
npm install
npm run dev        # http://localhost:4321  (syncs docs, then starts Astro)
```

## Build / preview the static site

```sh
npm run build      # outputs to website/dist
npm run preview    # serve the built site locally
```

## Add a page

1. Add the markdown to `../docs/`.
2. Register it in `scripts/sync-docs.mjs` (`PAGES` + `URL_MAP`).
3. Add it to the sidebar in `src/data/docsNav.ts`.

The sync step fails the build if a sidebar link has no page, so the two cannot
drift apart.
