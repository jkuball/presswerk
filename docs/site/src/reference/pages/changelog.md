`prepare` writes one version section into the declared changelog and commits it with the version
files. That committed section is the frozen release notes used by publishers and by retries of the
same release commit.

## Path

[`changelog.path`](/reference/options/#changelog.path) is the repository-relative changelog file.
It defaults to `CHANGELOG.md`.

## Template

[`changelog.template`](/reference/options/#changelog.template) is a MiniJinja string. Its default is
Presswerk's built-in Markdown template, so adding this option does not change existing output. Read
a template file with `builtins.readFile`:

```nix presswerk-module
{
  changelog.template = builtins.readFile ./release-notes.md.j2;
}
```

The context is the same for every commit analyzer:

- `version`: the version being prepared, as a string.
- `groups`: the ordered release-note groups.
  - `title`: the group heading.
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
