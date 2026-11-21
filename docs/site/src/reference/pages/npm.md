The npm publisher publishes a package tarball built by the flake to an npm registry, such as the
public npm registry, GitHub Packages, or a self-hosted Verdaccio. Declare it under
[`publishers.npm`](/reference/options/#publishers.npm), keyed by a name of your choice:

```nix presswerk-module
{
  publishers.npm.default = {
    name = "@scope/package";
    package = self.packages.x86_64-linux.npm-tarball;
    registry = "https://registry.npmjs.org";
    access = "public";
    tag = "presswerk";
    aliases = [ "latest" ];
    token.env = "NPM_TOKEN";
  };
}
```

`presswerk plan` lists the version it publishes, the dist-tag it uploads it with, and the aliases:

```console
publishers
  npm.default  https://registry.npmjs.org/@scope/package
    version  @scope/package@1.2.0
    tag      presswerk
    alias    latest
```

`registry` and `access` have no default, so a package never lands on a public registry, or becomes
public, by accident.

## Package

`package` is the finished tarball, as `npm pack` writes it, or a directory that holds exactly one
`.tgz`. Presswerk never packs and publishes the tarball as it is. It reads only the tarball's
`package.json`, which the upload needs: its `name` must equal `name`, and its `version` the release
version, or Presswerk stops before it writes anything. Nothing else in the package is checked; the
registry rejects what it rejects.

npm never accepts a version twice, not even after it was unpublished. Build the tarball
reproducibly, so a retry finds the same bytes that the first attempt uploaded. `npm pack` writes
every entry with a fixed modification time, so a derivation like this one gives the same tarball
for the same files:

```nix
packages.npm-tarball = pkgs.runCommand "package-npm-tarball" {
  nativeBuildInputs = [ pkgs.nodejs ];
} ''
  cp -r ${./package} package
  chmod -R u+w package
  mkdir $out
  cd package
  HOME=$TMPDIR npm pack --ignore-scripts --pack-destination $out
'';
```

Check it with `nix build --rebuild`, which fails if a second build gives other bytes. Keep the
version in `package.json` among the release's [version files](/reference/version-files/), so it is
always the release version:

```nix presswerk-module
{
  versionFiles.json.package = {
    path = "package/package.json";
    key = "version";
  };
}
```

## Tag and aliases

Every npm publish sends one dist-tag with the version; the npm client defaults to `latest`.
Presswerk makes the choice explicit with `tag`. It is set together with the upload and never
moved afterwards: a rerun for a version that already exists does not touch it, and a tag that has
since moved to another version is not a conflict.

`aliases` are dist-tags that follow the release line, such as `latest` or `release-{major}`. They
are templates like the [OCI image aliases](/reference/publishers/oci-image/#tags-and-aliases):
`{major}`, `{minor}`, and `{patch}` are replaced by the parts of the version, while `{version}` is
not allowed. An alias moves only to a higher final version than the one it points to. After
`2.0.0`, a maintenance release `1.4.1` moves `release-1`, while `latest` stays at `2.0.0`. An alias
left alone is reported as `kept`. Presswerk sets aliases through the registry's dist-tag API after
the upload, and a failure there is a warning, not a failed release.

The recommended setup is a dedicated tag with `latest` as an alias:

```nix presswerk-module
{
  publishers.npm.default = {
    # …
    tag = "presswerk";
    aliases = [ "latest" ];
  };
}
```

`latest` then only ever moves forward. `tag = "latest";` without aliases works too, including with
registries that have no dist-tag API, but the upload then always sets `latest`: publishing an older
version, such as a maintenance release or a republish, moves `latest` back to it on its first
upload.

npm refuses dist-tags that read as a version or a range, such as `1.2` or `v1`, as the
[npm documentation](https://docs.npmjs.com/cli/commands/npm-dist-tag#caveats) explains. Module
evaluation rejects such a `tag`, an alias that renders to one, and a `tag` that is also an alias.

## Behavior

Before anything is written, Presswerk reads the package from the registry and compares the
release's version with the tarball. The registry's `dist.integrity`, the tarball's SHA-512, decides:

- **Missing:** the version does not exist. Presswerk uploads it.
- **Matching:** the version exists with the same integrity. It is left as it is.
- **Conflicting:** the version exists with another integrity. Presswerk stops before the Git tag is
  pushed and never overwrites it.
- **Unknown:** the registry cannot be read, or reports no SHA-512 integrity for the version.
  Presswerk stops.

If the upload fails, Presswerk reads the version again: a concurrent run that uploaded the same
tarball completes the release. After the upload, it waits until the registry shows the version,
then moves the aliases.

A release's package is complete once its version exists in the registry. The integrity is not
compared, the tag and the aliases do not count. Before `plan` and `prepare` start the next
release, they check this for every npm publisher of the previous release. They read with the
credentials if they are set, and anonymously otherwise, as a public package allows.

## Registries and credentials

Set either `token`, read from an environment variable at run time, or `username` and `password`
for basic authentication. Tokens and passwords are always secrets; a username that is no secret can
be given as `username.value`.

- **npm:** `registry = "https://registry.npmjs.org";` with an automation or granular access token,
  such as `token.env = "NPM_TOKEN";`. Unscoped packages are always public, so `access = "restricted";` is for scoped ones.
- **GitHub Packages:** `registry = "https://npm.pkg.github.com";` with `token.env = "GITHUB_TOKEN";`
  in GitHub Actions. The workflow needs the `packages: write` permission, and the package's scope
  must be the repository's owner.
- **Verdaccio:** a local registry for trying out a release, such as
  `registry = "http://localhost:4873";` with `username` and `password` of a Verdaccio user.

Registries without the dist-tag API, possibly Google Artifact Registry and GitHub Packages, leave
the aliases unset with a warning. Use `tag = "latest";` without aliases there.

## Options

<!-- reference:options -->
