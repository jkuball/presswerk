# What the OCI image and Helm publishers share. The leading underscore keeps import-tree from
# loading this file as a flake module; the type modules import it.
{ lib, publisherLib }:
let
  inherit (lib) mkOption types;
in
{
  registryOptions = what: {
    repository = mkOption {
      # A plain host and optional port: no credentials, query, or fragment in the declaration.
      type = types.strMatching "(([A-Za-z0-9-]+[.])+[A-Za-z0-9-]+(:[0-9]+)?|[A-Za-z0-9.-]+:[0-9]+|localhost|\\[[0-9A-Fa-f:]+](:[0-9]+)?)/[a-z0-9._-]+(/[a-z0-9._-]+)*";
      example = "ghcr.io/owner/project";
      description = "Registry host, with an optional port, and repository path the ${what} are pushed to. Credentials never go here, only into `username` and `password`. Loopback registries such as `localhost:5000` use plain HTTP, every other registry HTTPS.";
    };

    public = mkOption {
      type = types.bool;
      default = false;
      description = "Whether anyone may pull the ${what} without credentials. `plan` and `prepare` then check the previous release anonymously and never read `username` or `password`; otherwise they need the credentials for that check. `publish` always needs the credentials.";
    };

    username = mkOption {
      type = publisherLib.credential { what = "registry username"; valueExample = "oauth2accesstoken"; };
      example = { env = "REGISTRY_USERNAME"; };
      description = "Registry username: `{ env = \"NAME\"; }` reads it from an environment variable at run time; `{ value = \"…\"; }` writes it into the release declaration, for registries where it is no secret, such as `oauth2accesstoken` for Google Artifact Registry.";
    };

    password = mkOption {
      type = publisherLib.credential { what = "registry password or token"; secretOnly = true; };
      example = { env = "REGISTRY_TOKEN"; };
      description = "Registry password or token, such as `{ env = \"REGISTRY_TOKEN\"; }`, read from an environment variable at run time.";
    };
  };

  # What the declaration holds about the registry and its credentials.
  registryDeclaration = config: { inherit (config) repository public username password; };

  # A claim on one tag template of the repository; image and chart tags share one namespace.
  claim = repository: tag: {
    destination = "oci://${repository}";
    key = tag;
  };

  # Renders a tag template like the CLI does, with a version whose parts differ.
  renderTag = publisherLib.renderTemplate;

  # Rendered with a stand-in version, a tag must be a valid OCI tag.
  tag = types.addCheck types.nonEmptyStr
    (
      template: builtins.match "[A-Za-z0-9_][A-Za-z0-9_.-]{0,127}" (builtins.replaceStrings [ "{version}" "{major}" "{minor}" "{patch}" ] [ "0.0.0" "0" "0" "0" ] template) != null
    ) // { description = "OCI tag, optionally containing {version}, {major}, {minor}, or {patch}"; };
}
