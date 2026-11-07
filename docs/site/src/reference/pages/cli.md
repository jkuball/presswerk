Run the CLI from the Presswerk input locked in your flake, so it matches the `lib.mkRelease`
that evaluates your declaration:

```console
$ nix run --inputs-from . presswerk -- plan
```

Every command works on the repository in the current directory and reads the release
declaration from the `presswerk.default` output of its flake at an exact commit. Errors are
printed to standard error and exit with status 1.

<!-- reference:commands -->
