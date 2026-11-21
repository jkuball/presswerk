# What publishers with aliases share, such as OCI tags or npm dist-tags that follow a line of
# versions. The leading underscore keeps import-tree from loading this file as a flake module; the
# type modules import it.
{ lib }:
{
  # The rule every alias follows, for option descriptions.
  rule = "An alias may use `{major}`, `{minor}`, and `{patch}`, but not `{version}`. It moves only to a higher final version than the one it points to, so `{major}` stays within its major version.";

  # Rejects alias templates that name a single version; `hint` says where those belong instead.
  assertion = { address, aliases, hint }: {
    assertion = !lib.any (lib.hasInfix "{version}") aliases;
    message = "publishers.${address}.aliases must not contain {version}, which names a single version; ${hint}";
  };
}
