The OCI publisher pushes container images built with Nix to any registry that speaks the
[OCI distribution API](https://github.com/opencontainers/distribution-spec), such as GitHub
Container Registry, Google Artifact Registry, or Docker Hub. Declare it under
[`publishers.oci`](/reference/options/#publishers.oci), keyed by a name of your choice, with the
image for each platform:

```nix presswerk-module
{
  publishers.oci.default = {
    repository = "ghcr.io/owner/project";
    platforms."linux/amd64" = self.packages.x86_64-linux.image;
    platforms."linux/arm64" = self.packages.aarch64-linux.image;
    tags = [ "{version}" ];
    aliases = [ "latest" "{major}.{minor}" ];
    public = true;
    username.secret = "GITHUB_ACTOR";
    password.secret = "GITHUB_TOKEN";
  };
}
```

Container images are the only content the OCI publisher supports so far; publishing Helm charts,
which registries store as OCI artifacts as well, is planned.

`presswerk plan` lists the publisher with its platforms and the tags it will push:

```console
publishers
  oci.default  ghcr.io/owner/project
    image    linux/amd64
    image    linux/arm64
    tag      1.2.0
    alias    latest
    alias    1.2
```

## Images

Each image is a Docker image archive in the layout `dockerTools` writes, which may be compressed
with gzip or zstd, such as the output of `dockerTools.buildImage` or `dockerTools.buildLayeredImage`,
or an executable that writes an uncompressed archive to stdout, such as the output of
`dockerTools.streamLayeredImage`. Like a
release asset, `publish` fetches every image from the local store or a substituter before it writes
anything, and never builds one. An image is read only while it is pushed: each layer is compressed
and uploaded as it is read, so at most one compressed layer is on disk at a time. A streamed image
keeps the image out of the Nix store and binary caches, but does not make `publish` faster, since
every layer is still read and compressed.

The platform key, such as `linux/amd64` or `linux/arm/v7`, must match the operating system,
architecture, and variant in the image's config, so an image cannot end up under the wrong
platform. Presswerk checks this while it pushes the image, after the release tag is pushed: a wrong
key writes no manifest or tag, but stops a release whose tag already exists, so the fix is a new
release. The image's name and tag inside the archive are ignored.

Presswerk compresses every layer and writes the OCI manifests itself. Layers are compressed with
gzip, which every client can pull. With `compression.zstd.enable = true;` they are compressed with
zstd instead, which is smaller and faster to unpack, but needs Docker 23, containerd 1.5, or a
similarly recent client to pull.

The images of all platforms form one OCI image index, even if there is only one. An index lists one
manifest per platform behind a single tag, and a client pulls the manifest for its own platform.
The index is annotated with the version as `org.opencontainers.image.version`, and each of its
manifests with the store path of its image as `dev.presswerk.store-path`.

## Tags and aliases

Both `tags` and `aliases` point to the index, and both are templates: `{version}` is replaced by
the release version, and `{major}`, `{minor}`, and `{patch}` by its parts.

- **`tags`** belong to the release, such as `1.2.0` or `v1.2.0-alpine`, and are never moved. Each
  contains `{version}`, so every release has tags of its own. The default is `[ "{version}" ]`.
- **`aliases`** are what is often called floating or rolling tags, such as `latest`, `1`, or
  `1.2`. They follow the highest stable version and must not contain `{version}`. The default is
  no aliases.

Presswerk moves an alias only to a higher stable version than the one it points to, as read from
the version annotation of that index. This also keeps `{major}` and `{major}.{minor}` within
their line: after `2.0.0`, a maintenance release `1.4.1` moves `1` and `1.4`, while `latest`
stays at `2.0.0`. A pre-release moves no alias, and an alias that points to an index without a
version annotation was pushed by something else and stays as well. An alias left alone is reported
as `kept`.

A version with SemVer build metadata such as `1.2.0+build.5` cannot be published yet, since OCI
tags do not allow `+`.

## Behavior

Before anything is written, Presswerk compares every tag with this release. An index belongs to
the release when it carries the release version and the same store path for every platform; the
layers themselves are not compared, so a retry recognizes the release even if its layers were
compressed differently.

- **Missing:** the tag does not exist, or an alias points to a lower version. Presswerk pushes it.
- **Matching:** the tag already points to an index of this release. It is left as it is.
- **Conflicting:** a version tag, or an alias for the same version, points to an index with other
  images. Presswerk stops before the release tag is pushed and never overwrites it.
- **Unknown:** the registry does not answer whether a version tag exists, such as with a server
  error. Presswerk stops.

Aliases are written last and are best effort, as described under
[mutable aliases](https://github.com/jkuball/presswerk/blob/main/docs/conception/publication.md#mutable-aliases).
An alias that cannot be read or moved does not fail the release: once the version tags exist, the
images count as published, `publish` continues with the next publishers, and the alias row reports
the reason:

```console
  alias     ghcr.io/owner/project:latest  not moved: could not push ghcr.io/owner/project:latest: HTTP 500; rerun to repair
```

`publish` then also prints a warning. The next run of the release moves the alias.

When a tag is missing and no other tag points to an index of this release yet, Presswerk reads
each image, uploads every layer and config the registry does not have yet, and puts each
platform's manifest and the index by digest. It then points the version tags and finally the
aliases at the index. Right before it writes a tag, it inspects that tag again. A
write that fails, for example because the response was lost, is inspected once more: it counts as
done only if the tag now points to this index, so two runs of the same release at the same time
both succeed. Registries offer no conditional tag update, so a different image pushed under the
same tag in the moment between the last inspection and the write is overwritten; a later run
reports it as a conflict.

A release's images are complete once every version tag points to an index whose
`org.opencontainers.image.version` annotation is the release version. Aliases do not count, since
they are best effort and may already point to a later release. Before `plan` and `prepare` start the
next release, they check this for every OCI publisher of the previous release, without fetching
images or comparing digests; a missing tag or a different annotation stops them, so a release
whose `publish` broke off is rerun first.

A rerun of a complete release only inspects the tags and reports `already pushed`, without
reading its images. A tag that was deleted is restored on the next run, pointing at the index
the other tags still point to, or without uploading layers the registry still has. GitHub
Container Registry restores a deleted version together with its tags as soon as its index is
pushed again.

## Credentials

`username.secret` and `password.secret` name the environment variables that hold the registry
username and password or token. Neither enters the declaration or the Nix store; Presswerk reads
them at run time, stops if one is unset or empty, and never prints them. They answer the registry's
authentication challenge, either with HTTP basic authentication or with the bearer token flow that
most registries use.

A username that is no secret can be given as `username.value` instead, such as
`oauth2accesstoken` for Google Artifact Registry. It is written into the declaration like any
other option. The password always comes from an environment variable.

`publish` always needs the credentials. `plan` and `prepare` only read the previous release's tags,
and how they read depends on `public`:

- `public = true`: anyone may pull the images, so the tags are read anonymously, with a token for
  pull access only. The credential variables are never read, even if they are set. If the registry
  refuses the anonymous read, Presswerk stops and reports that the repository does not seem to be
  publicly readable.
- `public = false`, the default: the tags are read with the credentials, so `plan` and `prepare`
  need them as soon as a previous release declared this publisher.

- **GitHub Container Registry:** `GITHUB_ACTOR` and `GITHUB_TOKEN` in GitHub Actions. The workflow
  needs the `packages: write` permission.
- **Google Artifact Registry:** `username.value = "oauth2accesstoken";` and an access token, such
  as from `gcloud auth print-access-token`.

The repository must start with the registry host and an optional port, and must not contain
credentials such as `user:token@`, a query, or a fragment. Loopback registries such as
`localhost:5000/project` are reached over plain HTTP, every other registry over HTTPS.

## Options

<!-- reference:options -->
