Any build of the CLI can run these commands, as long as it reads the
[schema version](/reference/declaration-schema/) of your declaration. Before Presswerk 1.0, a CLI
reads only its own schema version; from 1.0 on, newer CLIs will also read older declarations. We
recommend running the CLI from the Presswerk input locked in your flake, so it always matches the
`lib.mkRelease` that evaluates your declaration:

```console
$ nix run --inputs-from . presswerk -- plan
```

Every command works on the repository in the current directory and reads the release declaration
from the `presswerk.default` output of its flake at an exact commit. Presswerk invokes `nix` from
`PATH` and uses `nix-store` there when `publish` substitutes artifacts. The flake package used by
`nix run` does not bundle or select another Nix version. There is no separate implementation or
version check: a CLI with the required flake and command capabilities works, while missing
capabilities fail with Nix's own diagnostic. Errors are printed to standard error and exit with
status 1.

<!-- reference:commands -->
