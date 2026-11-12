
## OCI images

The OCI publisher pushes container images built with Nix to any registry that speaks the
[OCI distribution API](https://github.com/opencontainers/distribution-spec), such as GitHub
Container Registry, Google Artifact Registry, or Docker Hub. Declare it under
[`publishers.oci`](/reference/options/#publishers.oci), keyed by a name of your choice, with one
image per platform:

```nix presswerk-module
{
  publishers.oci.default = {
    repository = "ghcr.io/owner/project";
    images."linux/amd64" = self.packages.x86_64-linux.image;
    images."linux/arm64" = self.packages.aarch64-linux.image;
    tags = [ "{version}" "latest" ];
    username.secret = "GITHUB_ACTOR";
    password.secret = "GITHUB_TOKEN";
  };
}
```

`presswerk plan` lists the publisher with its platforms and the tags it will push:

```console
publishers
  oci.default  ghcr.io/owner/project
    image    linux/amd64
    image    linux/arm64
    tag      1.2.0
    alias    latest
  github-release.default  owner/project
    release  v1.2.0
```

### Images

Each image is the output of `dockerTools.buildImage` or `dockerTools.buildLayeredImage`, a Docker
image archive that may be compressed with gzip or zstd, or of `dockerTools.streamLayeredImage`, a
script that writes such an archive. Like a release asset, `publish` fetches every image from the
local store or a substituter before it writes anything, and never builds one. A streamed image is
produced by running its script after it was fetched.

```nix
perSystem = { pkgs, self', ... }: {
  packages.image = pkgs.dockerTools.streamLayeredImage {
    name = "project";
    contents = [ self'.packages.default ];
    config.Cmd = [ "/bin/project" ];
  };
};
```

The platform key, such as `linux/amd64` or `linux/arm/v7`, must match the operating system,
architecture, and variant in the image's config, so an image cannot end up under the wrong
platform. The image's name and tag inside the archive are ignored.

Presswerk pushes every layer uncompressed, exactly as the archive holds it, and writes the OCI
manifests itself. The images of all platforms form one OCI image index, even if there is only one,
annotated with the version as `org.opencontainers.image.version`. Because nothing is recompressed,
the same images always produce byte-identical manifests and the same index digest: a retry
recognizes its own earlier push. Uncompressed layers take more space and bandwidth than
compressed ones; registries that deduplicate layers store each one only once.

### Tags

`tags` lists the tags that point to the index; `{version}` is replaced by the release version.
At least one tag must contain `{version}`, so every release has a tag of its own. A version with
SemVer build metadata such as `1.2.0+build.5` cannot be published yet, since OCI tags do not
allow `+`.

- A **version tag** contains `{version}`, such as `1.2.0` or `v1.2.0-alpine`. It belongs to this
  release and is never moved.
- An **alias** has no `{version}`, such as `latest`. It points to the highest stable version, as
  read from the version annotation of the index it points to. Presswerk moves it only to a higher
  stable version, so publishing an older maintenance release or a pre-release leaves it alone and
  reports `kept`. An alias that points to an index without a version annotation was pushed by
  something else and is kept as well.

### Behavior

Before anything is written, Presswerk compares every tag with the index digest of this release:

- **Missing:** the tag does not exist, or an alias points to a lower version. Presswerk pushes it.
- **Matching:** the tag already points to exactly this index. It is left as it is.
- **Conflicting:** a version tag, or an alias for the same version, points to another digest.
  Presswerk stops before the release tag is pushed and never overwrites it.
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

When a tag is missing, Presswerk uploads every layer and config the registry does not have yet,
puts each platform's manifest and the index by digest, and then points the version tags and
finally the aliases at the index. Right before it writes a tag, it inspects that tag again. A
write that fails, for example because the response was lost, is inspected once more: it counts as
done only if the tag now points to this index, so two runs of the same release at the same time
both succeed. Registries offer no conditional tag update, so a different image pushed under the
same tag in the moment between the last inspection and the write is overwritten; a later run
reports it as a conflict.

A rerun of a complete release only inspects the tags and reports `already pushed`. A tag that
was deleted is restored on the next run, without uploading layers the registry still has. GitHub
Container Registry restores a deleted version together with its tags as soon as its index is
pushed again.

### Credentials

`username.secret` and `password.secret` name the environment variables that hold the registry
username and password or token. Neither enters the declaration or the Nix store; Presswerk reads
them at run time, stops if one is unset or empty, and never prints them. They answer the registry's
authentication challenge, either with HTTP basic authentication or with the bearer token flow that
most registries use.

- **GitHub Container Registry:** `GITHUB_ACTOR` and `GITHUB_TOKEN` in GitHub Actions. The workflow
  needs the `packages: write` permission.
- **Google Artifact Registry:** the username `oauth2accesstoken` and an access token, such as from
  `gcloud auth print-access-token`.

The repository must start with the registry host and an optional port, and must not contain
credentials such as `user:token@`, a query, or a fragment. Loopback registries such as
`localhost:5000/project` are reached over plain HTTP, every other registry over HTTPS.

### Options

<!-- reference:oci-options -->
