`prepare` writes one version section into the declared changelog and commits it with the version
files. Publishers use that section as the release notes.

The new section goes above the newest version section. When the changelog has no heading in the
declared [heading](#heading) format, such as one written by another tool, the section goes above its
first level-2 heading of any format, or at the end when it has none. A missing changelog is created
with a `# Changelog` title.

## Path

[`changelog.path`](/reference/options/#changelog.path) is the repository-relative changelog file.
It defaults to `CHANGELOG.md`.

## Heading

[`changelog.heading`](/reference/options/#changelog.heading) is the version heading of a section.
It is a plain replacement template, not MiniJinja, and defaults to `## [{version}] - {date}`:

```nix presswerk-module
{
  changelog.heading = "## v{version} ({date})";
}
```

The heading is one line that starts with `#`. It contains `{version}` exactly once and `{date}` at
most once; other text is literal. The same format renders new headings and finds existing ones, so a
changelog written by another tool in that format keeps it:

```markdown
# CHANGELOG

## v0.39.0 (2026-11-20)
…

## v0.38.2 (2026-11-10)
…
```

`prepare` refuses a version that already has a heading. Publishers read the section under the
release version's heading, up to the next heading of the same level or the next version heading.

## Template

[`changelog.template`](/reference/options/#changelog.template) is a MiniJinja string. Its default is
Presswerk's built-in Markdown template. Read a template file with `builtins.readFile`:

```nix presswerk-module
{
  changelog.template = builtins.readFile ./release-notes.md.j2;
}
```

The context is the same for every commit analyzer:

- `version`: the version being prepared, as a string.
- `groups`: the release-note groups, ordered by level (`rank`), then by descending size, then by
  name.
  - `title`: the group heading.
  - `level`: the group's highest effective level: `"major"`, `"minor"`, `"patch"`, or `none`
    (`null`). A breaking commit counts as `"major"`.
  - `rank`: the numeric level order: `0` for major, `1` for minor, `2` for patch, and `3` for
    none.
  - `commits`: the ordered commits in the group.
    - `text`: the analyzer's ready-to-display line, without the list marker.
    - `message`: the parsed commit description without its intention or scope.
    - `scope`: the parsed scope as a string, or `none`.
    - `breaking`: whether the commit is a breaking change.
    - `id`: the full Git object ID.

For example:

```jinja
Release {{ version }}

{% for group in groups %}
### {{ group.title }}

{% for commit in group.commits %}
- {{ commit.text }} ([{{ commit.id[:7] }}](https://github.com/owner/project/commit/{{ commit.id }}))
{% endfor %}
{% endfor %}
```

Template syntax, unavailable filters, and unknown context fields fail declaration validation during
`plan` or `prepare`. A rendering error stops before a release commit is written.

## Options

<!-- reference:options -->
