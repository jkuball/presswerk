The Helm publisher pushes a chart archive to an OCI registry in the format Helm understands.
Declare it under [`publishers.helm`](/reference/options/#publishers.helm), keyed by a name of your
choice:

```nix presswerk-module
{
  publishers.helm.default = {
    repository = "ghcr.io/owner/charts/project";
    chart = self.packages.x86_64-linux.chart;
    public = true;
    username.secret = "GITHUB_ACTOR";
    password.secret = "GITHUB_TOKEN";
  };
}
```

`repository` is the full registry path including the chart name. It starts with the registry host
and an optional port, and its last segment must equal `name` in the packaged chart's `Chart.yaml`.
It must not contain credentials such as `user:token@`, a query, or a fragment. Loopback registries
such as `localhost:5000/charts/project` are reached over plain HTTP, every other registry over
HTTPS.

`chart` is the `.tgz` archive written by `helm package`. It must be a file in a flake output.
`publish` fetches it from the local Nix store or a substituter without building it.

`presswerk plan` shows the one chart tag:

```console
publishers
  helm.default  ghcr.io/owner/charts/project
    tag      ghcr.io/owner/charts/project:1.2.0
```

A first publication reports the tag as `pushed`; a retry of the same chart reports `already pushed`.

## Version

The chart's `version` must equal the release version. Presswerk opens the archive and checks the
chart name and version before it pushes the Git tag, so a mismatched package leaves no release
state behind. There is no separate chart version option.

Keep `version` and, when it tracks the application release, `appVersion` in `Chart.yaml` among the
release's [version files](/reference/version-files/). Today they use regex writers:

```nix presswerk-module
{
  versionFiles.regex.chartVersion = {
    path = "chart/Chart.yaml";
    pattern = ''^version: (?<version>\S+)$'';
  };
  versionFiles.regex.chartAppVersion = {
    path = "chart/Chart.yaml";
    pattern = ''^appVersion: (?<version>\S+)$'';
  };
}
```

The flake cannot read regex version files, so also declare a version file it can read. Native YAML
version files are planned in [#102](https://github.com/jkuball/presswerk/issues/102).

The chart is pushed under exactly one tag, the release version. Following Helm's convention, `+`
in SemVer build metadata becomes `_` in the registry tag, so `1.2.0+build.5` becomes
`1.2.0_build.5`.

## OCI artifact

Presswerk pushes one OCI image manifest, not an image index. Its config uses
`application/vnd.cncf.helm.config.v1+json`; its single chart layer uses
`application/vnd.cncf.helm.chart.content.v1.tar+gzip`. The chart archive is uploaded unchanged.
There are no platforms, extra tags, aliases, or compression settings.

The manifest records the release version as `org.opencontainers.image.version` and the chart's Nix
store path as `dev.presswerk.origin`. The store path identifies everything the chart was built from.

## Behavior

Before anything is written, Presswerk inspects the version tag:

- **Missing:** the tag does not exist. Presswerk uploads the config and chart blobs, the manifest,
  and the tag.
- **Matching:** the tag points to a Helm manifest with this version and the same store path. It is
  left as it is and reported as `already pushed`.
- **Conflicting:** the tag points to another store path, version, or kind of manifest. Presswerk
  stops before the Git tag is pushed and never overwrites it.
- **Unknown:** the registry cannot say what the tag points to. Presswerk stops.

A release's chart is complete once its tag points to its version and store path. Before `plan` and
`prepare` start the next release, they check the previous release's own Helm publishers. A missing
or different chart stops them, so the interrupted release is published again first.

## Credentials

Helm publishers use the same registry authentication as [OCI image publishers](/reference/publishers/oci/#credentials).
`username.secret` and `password.secret` name environment variables read at run time. A public
username may instead use `username.value`, such as `oauth2accesstoken` for Google Artifact
Registry. Credentials never enter the declaration or Nix store and are never printed.

`publish` always needs the credentials. By default, `plan` and `prepare` need them when they check a
previous release. With `public = true;`, that check reads anonymously with pull access only and
never reads the credential variables.

## Options

<!-- reference:options -->
