Any build of the CLI can run these commands, as long as it reads the schema version of your
declaration. We recommend running it from the Presswerk input locked in your flake, so it always
matches the `lib.mkRelease` that evaluates your declaration:

```console
$ nix run --inputs-from . presswerk -- plan
```

Every command works on the repository in the current directory and reads the release
declaration from the `presswerk.default` output of its flake at an exact commit. Errors are
printed to standard error and exit with status 1.

<!-- reference:commands -->
