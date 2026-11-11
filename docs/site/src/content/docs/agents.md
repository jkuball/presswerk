---
title: Agents
description: What coding agents and other automated clients can read from these docs, and what they cannot.
---

These docs are meant to be read by coding agents and other automated clients as well as by people.
Every page carries the same information in both forms, including the generated reference.

:::caution
Before Presswerk 1.0, the URLs and formats described here may change with any Presswerk version.
:::

## What works

- **Index.** [`/llms.txt`](/llms.txt) lists every page with a short description and links to its
  Markdown copy.
- **Markdown copies.** Every page is also published as Markdown at the same path with `.md`, for
  example [`/guide/walkthrough.md`](/guide/walkthrough.md) for the walkthrough. Links between pages
  in these copies point to other Markdown copies, so an agent that follows them stays in Markdown.
- **Discovery.** Every HTML page links to its Markdown copy with
  `<link rel="alternate" type="text/markdown">` and to the index with `<link rel="describedby">`.
- **No JavaScript needed.** Filters and search are added on top of complete static pages, so
  nothing essential depends on running scripts.
- **Crawling.** `robots.txt` allows all crawlers and points to the sitemap.

## What does not

- **Content negotiation.** Requesting a page with `Accept: text/markdown` still returns HTML. The
  docs are static files on GitHub Pages, which ignores request headers. Use the `.md` URL instead.
- **A single full-text file.** There is no `llms-full.txt` yet. It was left out to keep maintenance
  low and can be added if it turns out to be needed.
- **Older versions.** The docs always describe the latest release. An agent working with an older
  Presswerk version should read the docs in that version's source tree under `docs/site`.
