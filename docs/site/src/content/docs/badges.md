---
title: Badges
description: Show in your README that your project releases with Presswerk, and which version it is at.
---

Two badges show Presswerk in your README: **Released with Presswerk** says how your project
releases, and the **version badge** shows its current version in the same style.

<div class="pw-badge-previews">
  <p class="pw-badge-preview pw-badge-preview-light">
    <img alt="Released with Presswerk" src="/badge.svg" width="164" height="20">
    <img alt="Version badge" src="/version-badge-light.svg" height="20">
  </p>
  <p class="pw-badge-preview pw-badge-preview-dark">
    <img alt="Released with Presswerk" src="/badge-light.svg" width="164" height="20">
    <img alt="Version badge" src="/version-badge-dark.svg" height="20">
  </p>
</div>

## Released with Presswerk

A fixed image in two variants: a dark label for light pages, and a light label for dark pages. It
does not change with your releases.

### Follow the colour scheme

GitHub renders `<picture>` in a README. This snippet shows the dark variant on light pages and
the light variant on dark pages. It stays on one line, because whitespace inside the link would show
as an underlined gap next to the badge:

```html title="README.md"
<a href="https://presswerk.dev/"><picture><source media="(prefers-color-scheme: dark)" srcset="https://presswerk.dev/badge-light.svg"><img alt="Released with Presswerk" src="https://presswerk.dev/badge.svg" width="164" height="20"></picture></a>
```

### One variant

Where HTML is not rendered, use one variant in Markdown:

```md title="README.md"
[![Released with Presswerk](https://presswerk.dev/badge.svg)](https://presswerk.dev/)
```

For the light variant, use `badge-light.svg` instead.

## Version badge

Presswerk keeps the version badge in your repository and writes the new version into it with every
release; declare it as a [version file](/reference/version-files/#badge). By default, one file
follows the viewer's system colour scheme:

```md title="README.md"
![Version](.github/presswerk-badge.svg)
```

The system colour scheme can differ from GitHub's theme. To follow GitHub's theme exactly, declare
one badge per [`colorScheme`](/reference/options/#versionFiles.badge.name.colorScheme) and choose
between them with `<picture>`:

```html title="README.md"
<picture>
  <source media="(prefers-color-scheme: dark)" srcset=".github/badge-dark.svg">
  <img alt="Version" src=".github/badge-light.svg" height="20">
</picture>
```
