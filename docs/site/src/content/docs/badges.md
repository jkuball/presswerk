---
title: Badges
description: Show in your README that your project releases with Presswerk.
---

Show in your README that your project releases with Presswerk. The badge comes in two variants: a
dark label for light pages, and a light label for dark pages.

![Released with Presswerk](/badge.svg) ![Released with Presswerk](/badge-light.svg)

## Follow the colour scheme

GitHub renders `<picture>` in a README. This snippet shows the dark variant on light pages and
the light variant on dark pages:

```html title="README.md"
<a href="https://presswerk.dev/">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://presswerk.dev/badge-light.svg">
    <img alt="Released with Presswerk" src="https://presswerk.dev/badge.svg" width="164" height="20">
  </picture>
</a>
```

## One variant

Where HTML is not rendered, use one variant in Markdown:

```md title="README.md"
[![Released with Presswerk](https://presswerk.dev/badge.svg)](https://presswerk.dev/)
```

For the light variant, use `badge-light.svg` instead.

The badges are fixed images. They do not change with your releases.

## Version badge

To show your current version in the same style, let Presswerk keep a
[version badge](/reference/version-files/#badge) in your repository:

![Version badge for Presswerk itself](/version-badge.svg)
