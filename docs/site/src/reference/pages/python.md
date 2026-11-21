The Python publisher uploads the wheels and the sdist of a release to a package index: PyPI or a
private index such as Google Artifact Registry, AWS CodeArtifact, devpi, or GitLab. Declare it
under [`publishers.python`](/reference/options/#publishers.python), keyed by a name of your choice:

```nix presswerk-module
{
  publishers.python.default = {
    name = "my-package";
    distributions = self.packages.x86_64-linux.my-package-dist;
    repositoryUrl = "https://upload.pypi.org/legacy/";
    indexUrl = "https://pypi.org/simple/";
    username.value = "__token__";
    password.env = "PYPI_TOKEN";
  };
}
```

`presswerk plan` shows the project and version the release publishes:

```console
publishers
  python.default  https://pypi.org/simple/
    release  my-package==1.2.0
```

A first publication reports each file as `uploaded`; a retry reports the files already on the index
as `already uploaded`.

## Indexes

Every index has two endpoints, which PyPI keeps apart and which are both required:

- `repositoryUrl` is the legacy upload API, which `twine` and `uv publish` also upload through.
- `indexUrl` is the simple API that Presswerk reads the release's state from, in its JSON
  ([PEP 691](https://peps.python.org/pep-0691/)) or HTML
  ([PEP 503](https://peps.python.org/pep-0503/)) form.

There is no default index. For PyPI, the upload API is `https://upload.pypi.org/legacy/` and the
simple API `https://pypi.org/simple/`; TestPyPI uses `https://test.pypi.org/legacy/` and
`https://test.pypi.org/simple/`. A private index names both in its documentation, such as a Python
repository in Google Artifact Registry:

```nix presswerk-module
{
  publishers.python.internal = {
    name = "my-package";
    distributions = self.packages.x86_64-linux.my-package-dist;
    repositoryUrl = "https://europe-west3-python.pkg.dev/my-project/my-index/";
    indexUrl = "https://europe-west3-python.pkg.dev/my-project/my-index/simple/";
    username.value = "oauth2accesstoken";
    password.env = "GOOGLE_ACCESS_TOKEN";
  };
}
```

One project version can only be released once per index, so two Python publishers with the same
`name` and `indexUrl` fail module evaluation, also when the URLs are spelled differently, such as
`https://PyPI.org:443/simple`. Publishing one project to two indexes, such as
PyPI and a private mirror, takes two publishers.

## Distributions

`distributions` is a directory in a flake output that holds the finished wheels (`.whl`) and the
sdist (`.tar.gz`), and no other files. Presswerk never builds them: the flake does, like every
other artifact, and `publish` fetches the directory from the local store or a substituter.

Indexes never replace a file, and PyPI refuses a filename forever, even after its deletion. A retry
is only safe if the distributions are reproducible byte for byte, so that the same commit always
builds the same files. Python build backends such as Hatchling, Flit, and uv honor
`SOURCE_DATE_EPOCH`, which the Nix stdenv sets. A derivation that builds both with
[build](https://build.pypa.io/):

```nix
my-package-dist = pkgs.stdenvNoCC.mkDerivation {
  pname = "my-package-dist";
  version = "1.2.0";
  src = ./.;
  nativeBuildInputs = [
    (pkgs.python3.withPackages (ps: [ ps.build ps.hatchling ]))
  ];
  # Writes my_package-1.2.0-py3-none-any.whl and my_package-1.2.0.tar.gz.
  buildPhase = ''
    python -m build --no-isolation --outdir $out .
  '';
  dontInstall = true;
  dontFixup = true;
};
```

[pyproject.nix](https://pyproject-nix.github.io/pyproject.nix/) and
[uv2nix](https://pyproject-nix.github.io/uv2nix/) builds work the same way.

## Version

Presswerk reads only what an upload needs: each file's name, its core metadata (`METADATA` in a
wheel, `PKG-INFO` in an sdist), and its SHA-256 digest. It compares the project name and version in
the filename and the metadata with `name` and the release version before it pushes the Git tag,
so a mismatched build leaves no release state behind. Nothing else is checked; the index decides
whether the metadata is valid. Its core metadata fields are sent with each upload unchanged, as
`twine` and `uv publish` send them.

Keep the version in `pyproject.toml` among the release's
[version files](/reference/version-files/):

```nix presswerk-module
{
  versionFiles.toml.pyproject = {
    path = "pyproject.toml";
    key = "project.version";
  };
}
```

Pre-release versions cannot be published yet, because
their SemVer and PEP 440 spellings differ, such as `1.4.0-rc.1` and `1.4.0rc1`.

## Behavior

Before anything is written, Presswerk lists the project's files on the simple API and compares each
local file with the file of the same name:

- **Missing:** the index does not list it. Presswerk uploads it.
- **Matching:** the index lists it with the same SHA-256 digest. It is left as it is and reported as
  `already uploaded`.
- **Conflicting:** the index lists it with another digest. Presswerk stops before the Git tag is
  pushed and never uploads anything, not even the other files.
- **Unknown:** the index lists it without a SHA-256 digest, or its listing cannot be read
  completely, so Presswerk cannot verify it. Presswerk stops as well.

The release is complete once every declared file matches. A wheel that failed to upload is
therefore uploaded on a retry, and the files already on the index are kept. If an upload fails
because another run uploaded the same file meanwhile, Presswerk accepts it once the index lists it
with the same digest.

An index may list a new file only some time after its upload. After its own uploads, Presswerk
waits up to two minutes for the simple API to list them; if it still does not, the run fails and a
retry completes it.

Before `plan` and `prepare` start the next release, they check the previous release's own Python
publishers without its files: the index must list at least one file of that version.

## Credentials

`publish` uploads with `username` and `password`. For PyPI, the username is `__token__` and the
password an [API token](https://pypi.org/help/#apitoken). Other indexes name their own, such as
`oauth2accesstoken` and an access token for Google Artifact Registry, or `_json_key_base64` and a
base64-encoded service account key.

Presswerk reads the simple API anonymously and sends the credentials there only when the index asks
for them, as pip does. So `plan` and `prepare` need no credentials for a public index such as PyPI,
and its upload token never reaches the simple API.

## Options

<!-- reference:options -->
