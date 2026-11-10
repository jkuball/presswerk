---
title: Badges
description: Show in your README that your project releases with Presswerk.
---

Show in your README that your project releases with Presswerk. The badge comes in two variants: a
dark label for light pages, and a light label for dark pages.

![Released with Presswerk](/presswerk/badge.svg) ![Released with Presswerk](/presswerk/badge-light.svg)

## Follow the colour scheme

GitHub renders `<picture>` in a README. This snippet shows the dark variant on light pages and
the light variant on dark pages:

```html title="README.md"
<a href="https://jkuball.github.io/presswerk/">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://jkuball.github.io/presswerk/badge-light.svg">
    <img alt="Released with Presswerk" src="https://jkuball.github.io/presswerk/badge.svg" width="164" height="20">
  </picture>
</a>
```

## One variant

Where HTML is not rendered, use one variant in Markdown:

```md title="README.md"
[![Released with Presswerk](https://jkuball.github.io/presswerk/badge.svg)](https://jkuball.github.io/presswerk/)
```

For the light variant, use `badge-light.svg` instead.

The badges are fixed images. They do not change with your releases.
