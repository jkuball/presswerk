# Changelog

## [Unreleased]

- 📝 Every release in the changelog starts with hand-written highlights and folds its commits
  below them.

## [0.23.1] - 2026-11-24

- 🐛 `publish` keeps every fetched artifact from garbage collection until it is done with it.

<details>
<summary>Commits (4)</summary>

- 📝 (agents): Add agent instructions
- 📝 (docs): Note the temporary GC roots of fetched artifacts
- 🐛 (artifacts): Keep fetched artifacts alive until publish is done
- 🔧 (release): Collapse commit groups in release notes

</details>

## [0.23.0] - 2026-11-23

- ✨ An `npm` publisher publishes a tarball built by the flake to an npm registry, with dist-tag
  aliases such as `latest` that only move forward.
- ✨ A `cargo` publisher uploads a crate built by the flake to crates.io or any sparse-index
  registry.
- ✨ A `python` publisher uploads the wheels and sdist built by the flake to PyPI or any other
  package index, and completes a partially uploaded release on retry.

<details>
<summary>Commits (31)</summary>

- ✨ (python): Declare Python publishers in the flake
- ✨ (python): Upload wheels and sdists to package indexes
- ✨ (cargo): Declare the crate name explicitly instead of deriving it from the instance
- ✨ (cargo): Add the Cargo publisher for sparse-index registries
- ✨ (npm): Add an npm publisher
- 📝 (docs): Note the Python publisher in the unreleased changes
- 📝 (docs): Explain the canonical index URL and the approximate wait
- 📝 (docs): Describe unverifiable Python files and canonical index URLs
- 📝 (docs): Document the Python publisher
- 📝 (docs): Drop the kellnr rustdoc note from the Cargo page
- 📝 (docs): Note the Cargo publisher as unreleased
- 📝 (docs): Name the crate with the explicit name option
- 📝 (conception): Let publishers read what publishing needs and never validate content
- 📝 (docs): Document the Cargo publisher
- 📝 (docs): List the npm publisher in the declaration schema and the docs checks
- 📝 (docs): List the built-in publishers without a count
- 📝 (docs): Note the npm publisher in the unreleased changes
- 📝 (docs): Document the npm publisher
- 🐛 (python): Require the canonical simple API URL
- 🐛 (python): Cross-check link names and scrub authenticated read errors
- 🐛 (python): Claim equivalent simple API URLs as one index
- 🐛 (python): Fail closed on unreadable listings and files, stalls, and credential echoes
- 🐛 (cargo): Validate the publish API as a base URL before appending the upload route
- 🐛 (cargo): Fail closed on unreadable index rows, bound index requests, check the publish API before the tag, and require canonical indexes
- 🐛 (npm): Decide pending aliases from live dist-tags and redact echoes before shortening
- 🐛 (npm): Claim canonical registries, follow npm's range grammar, and read live dist-tags
- 🚸 (cargo): Report a yanked matching version as kept instead of repeating a note
- ♻️ (python): Name the project with a name option like the other registry publishers
- ♻️ (publishers): Share the Base64 encoder
- ♻️ (publishers): Share the alias rule between publishers
- ✅ (cli): Use a publisher type that stays unknown in the predecessor test

</details>

## [0.22.0] - 2026-11-21

- ✨ A previous release that Presswerk cannot read is now trusted with the `trustedPredecessor`
  option in the release module, so `plan` passes and the usual `prepare` releases on top of it,
  without running `prepare --trust-predecessor` by hand.
- ✨ `plan` now accepts `--trust-predecessor` too.
- ✨ A `trustedPredecessor` that names an older release than the previous one now has no effect,
  and `plan` and `prepare` warn that it can be removed.

<details>
<summary>Commits (5)</summary>

- ✨ (release): Declare a trusted predecessor in the release module
- 📝 (docs): Note the trustedPredecessor option in the changelog
- 📝 (docs): Show the trustedPredecessor option before the flag
- 🐛 (release): Check stale trust against the predecessor and quote trust hints
- 🚸 (plan): Repeat the trust flag in the prepare hint

</details>

## [0.21.0] - 2026-11-21

- 💥 Credentials are declared as `{ env = "NAME"; }` instead of `{ secret = "NAME"; }`; usernames
  may still be `{ value = "…"; }`. This affects every publisher with credentials, including the
  GitHub Release token, which defaulted to `token.secret = "GITHUB_TOKEN"`. If the previous release
  used the old syntax, `plan` reports it as unreadable; once you have verified that it is complete,
  run `presswerk prepare --trust-predecessor <tag>=<commit>`, which `prepare` prints.
- 💥 `publish` now stops before writing anything if a declared secret is missing.
- ✨ Two publishers that claim the same tag or GitHub Release now stop the release before anything
  is published.

<details>
<summary>Commits (27)</summary>

- 💥 (release): Declare every credential as a value or a secret source
- 📝 (docs): Phrase the upgrade note as changes
- 📝 (docs): Classify the upgrade note with gitmojis
- 📝 (docs): Drop the links from the upgrade note
- 📝 (docs): Qualify the upgrade note
- 📝 (docs): Turn the unreleased notes into a short upgrade note
- 📝 (docs): Introduce the claim collision error
- 📝 (docs): Link the publisher examples to their options and trim claims
- 📝 (docs): Explain claims as a concept with one example
- 📝 (docs): Name the claim setups exactly
- 📝 (docs): Explain claims as a destination plus a version-template key
- 📝 (docs): Explain the publisher interface, claims, and credentials
- 🐛 (publish): Read secrets only from the credentials each type names
- 🐛 (publish): Report an unreadable previous release as unknown
- 🐛 (publish): Check every publisher at the released version and redact all output
- 🔒️ (oci): Keep publisher secrets from image executables
- 🔒️ (oci): Redact what image executables print
- 🔒️ (release): Accept tokens and passwords only as secrets and fold GitHub repository case in claims
- 🚸 (release): Name colliding claims as written, without the sample version
- 🚸 (prepare): Name why a previous declaration cannot be read
- ♻️ (release): Flatten credentials to one of value or env
- ♻️ (release): Derive claims as version templates in Nix and render them in the CLI
- ♻️ (release): Split the publisher options by type and check claims in Nix
- ♻️ (publish): Put the publisher types behind one interface
- ♻️ (publish): Move the publisher types into their own modules
- 🔥 (release): Read only the current credential shape
- ✅ (oci): Check the stripped secrets without changing the test environment

</details>

## [0.20.1] - 2026-11-19

- 🐛 `plan` and `prepare` build on a release that is committed but not yet published or whose tag
  is not fetched yet, instead of failing.

<details>
<summary>Commits (7)</summary>

- 🐛 (prepare): Skip release notes at a pending release commit
- 🐛 (plan): Build on a published release whose tag is not fetched yet
- 🐛 (prepare): Refuse a pending base before computing the next release
- 🐛 (plan): Leave out the prepare hint above a pending release
- 🐛 (prepare): Build on a pending release instead of failing
- 📝 (docs): Run prepare on the pending release commit
- 👷 (ci): Prepare on the release app's pushes too

</details>

## [0.20.0] - 2026-11-18

- ✨ The `stable` option declares whether the public API is stable; while it is false, a breaking
  change on `0.y.z` advances the minor version instead of releasing `1.0.0`.

<details>
<summary>Commits (8)</summary>

- ✨ (release): Add the stable API option
- 📝 (docs): Explain analyzer-specific examples
- 📝 (release): Explain the stable prerelease check
- 📝 (release): Clarify stable option compatibility
- 🐛 (declaration): Decode releases without stable
- ✅ (release): Restore workspace default coverage
- ✅ (release): Inline the workspace test fixture
- ✅ (release): Evaluate stable defaults in workspace tests

</details>

## [0.19.0] - 2026-11-18

- ✨ `publish` runs independent publishers at the same time, up to `--jobs`, and orders the others
  with the `after` and `before` options.
- 🐛 A failing publisher only skips the publishers that depend on it, and `publish` reports the
  partial result and exits with a failure status.
- ⚡️ The OCI publishers upload image layers in a pipeline.

<details>
<summary>Commits (18)</summary>

- ✨ (publish): Run publishers with bounded jobs
- ✨ (publish): Complete publisher graph semantics
- ✨ (publish): Schedule publisher dependency graph
- 🐛 (oci): Bound pipelined upload memory
- 🐛 (publish): Report panicking publishers without hanging
- 🐛 (publish): Align completion test with graph outcomes
- 🐛 (publish): Fix graph failure propagation
- 🐛 (publish): Return the partial failure status
- 🐛 (publish): Flush partial summary before exiting
- 📝 (oci): Document layer upload pipelining
- 📝 (publish): Place GitHub ordering with its publisher
- 📝 (publish): Present before as a normal option
- 📝 (publish): Trim recovery guidance
- ⚡️ (oci): Pipeline layer uploads
- ✅ (oci): Avoid timing-sensitive upload saturation
- ✅ (publish): Cover concurrent transitive skips
- ✅ (publish): Cover concurrent completion ordering
- ♻️ (publish): Centralize dependency rewriting

</details>

## [0.18.0] - 2026-11-17

- ✨ `publish` prints each step as soon as it completes instead of at the end.

<details>
<summary>Commits (1)</summary>

- ✨ (publish): Print each step as soon as it completes

</details>

## [0.17.1] - 2026-11-17

- 🐛 A GitHub Release whose notes exceed GitHub's size limit links to the changelog instead of
  failing.

<details>
<summary>Commits (1)</summary>

- 🐛 (github-release): Link the changelog when the body exceeds GitHub's limit

</details>

## [0.17.0] - 2026-11-17

- ✨ The OCI image publisher takes over the aliases of a trusted predecessor, and `plan` warns
  about aliases that `publish` keeps or takes over, and before it reads aliases without
  credentials.

<details>
<summary>Commits (12)</summary>

- ✨ (plan): Warn before reading aliases without credentials
- ✨ (plan): Warn about aliases that publish keeps or takes over
- ✨ (oci): Take over aliases of a trusted predecessor
- 📝 (adoption): Name the command that warns during an adoption
- 📝 (adoption): Move OCI image aliases to the end as a special case
- 📝 (oci): Describe alias reads without credentials
- 📝 (oci): Document taken-over aliases and the alias warnings
- 📝 (adoption): Describe how the first release takes over aliases
- 💬 (oci): Say that foreign aliases lack a version annotation
- ✅ (oci): Cover which credential variables are missing
- ✅ (oci): Cover finding foreign aliases before anything is written
- ✅ (oci): Cover the takeover of a trusted predecessor's aliases

</details>

## [0.16.1] - 2026-11-16

- 🐛 `publish` checks that Git has a tagger identity before it writes anything.

<details>
<summary>Commits (5)</summary>

- 📝 (conception): Recommend .agents/skills/ for installing the agent skill
- 📝 (conception): Deliver the agent skill as a skill subcommand with a repository-specific variant
- 🐛 (publish): Check the Git tagger identity during preflight
- 💚 (ci): Leave packages.default out of the build so the release link is stable
- ✅ (publish): Fail before writing when the Git identity is missing

</details>

## [0.16.0] - 2026-11-15

- ✨ Hand-written notes under `## [Unreleased]` move into the new version section, and `prepare`
  removes the unreleased section.

<details>
<summary>Commits (5)</summary>

- ✨ (changelog): Remove the unreleased section when its notes move into a release
- ✨ (changelog): Leave unreleased notes untouched when the template does not use them
- ✨ (changelog): Move hand-written unreleased notes into the new section
- ✅ (changelog): Keep comment lines of code blocks in unreleased notes
- ✅ (changelog): Cover an unreleased heading without version headings

</details>

## [0.15.0] - 2026-11-15

- ✨ The changelog's version heading format and its strftime date format are declarable.

<details>
<summary>Commits (10)</summary>

- ✨ (changelog): Declare a strftime date format in the heading
- ✨ (changelog): Declare the version heading format
- 🐛 (changelog): Reject date formats that break the heading line
- 🐛 (changelog): Refuse headings that split into more than one version and date
- 🐛 (changelog): Do not take inline code for a code fence
- 🐛 (changelog): Ignore heading lines inside fenced code blocks
- 🐛 (changelog): Refuse sections that do not read back under the heading
- 🐛 (changelog): Find headings with ISO dates and without trailing whitespace
- 📝 (changelog): Describe the heading placeholders without contrasting MiniJinja
- 📝 (changelog): Note that headings with other date formats are not recognized

</details>

## [0.14.0] - 2026-11-15

- ✨ A Cargo workspace version file updates the workspace version and its members in one lock
  file.

<details>
<summary>Commits (5)</summary>

- ✨ (version): Default a Cargo workspace's lock file to its manifest's directory
- ✨ (version): Add Cargo workspace version files
- 🐛 (version): Keep literal Cargo workspace members that an exclude covers
- 📝 (version): Rewrap the Cargo workspace docs
- ✅ (version): Prepare a Cargo workspace and an own-version member in one lock file

</details>

## [0.13.0] - 2026-11-15

- ✨ JSON and TOML version files select keys with quoted segments, such as `"tool.x".version`.

<details>
<summary>Commits (4)</summary>

- ✨ (version-files): Select JSON and TOML keys with quoted segments
- 🐛 (preparation): Write several keys of one version file into one release file
- 📝 (version-files): Link the TOML 1.0 key syntax
- ♻️ (version-files): Parse keys as TOML dotted keys in the flake and the CLI

</details>

## [0.12.0] - 2026-11-15

- ✨ `prepare` adopts a predecessor that has no Presswerk declaration and trusts lightweight tags,
  so an existing project can switch to Presswerk without a matching release.

<details>
<summary>Commits (8)</summary>

- ✨ (prepare): Trust a lightweight tag of any trusted predecessor
- ✨ (prepare): Adopt a predecessor without a Presswerk declaration
- 📝 (docs): Shorten the version file comment
- 📝 (docs): List how to adopt a project without a matching tag
- 📝 (docs): Note that any kind of version file fits for adoption
- 📝 (docs): Name the adoption commit without "Release"
- 📝 (docs): Adopt Presswerk with a commit that selects no version level
- 📝 (docs): State what adopting a predecessor checks

</details>

## [0.11.0] - 2026-11-15

- ✨ A changelog without version headings receives the new section before its first level-2
  heading.

<details>
<summary>Commits (1)</summary>

- ✨ (changelog): Insert before the first level-2 heading without a version heading

</details>

## [0.10.1] - 2026-11-15

- 🐛 Presswerk's own releases publish from the release app's push; nothing changes for users.

<details>
<summary>Commits (1)</summary>

- 🐛 (ci): Publish from the app's release push instead of dispatching

</details>

## [0.10.0] - 2026-11-15

- ✨ Changelog sections and GitHub Release bodies are rendered from a declarable MiniJinja
  template, with the note groups ordered by release level.
- 🐛 `publish` stops on a GitHub Release body that conflicts with the release and ignores later
  edits to a published one.

<details>
<summary>Commits (6)</summary>

- ✨ (release): Order note groups by effective level
- ✨ (release): Template changelog notes and GitHub Release bodies
- 🐛 (publish): Ignore published GitHub Release body edits
- 🐛 (publish): Reject GitHub Release body conflicts
- 📝 (docs): Clarify release-note group order
- 📝 (docs): Describe template behavior for users

</details>

## [0.9.0] - 2026-11-15

- 💥 The OCI publisher type `publishers.oci` is now `publishers.oci-image`.
- ✨ An `oci-helm` publisher pushes packaged Helm charts to OCI registries.

<details>
<summary>Commits (12)</summary>

- 🐛 (docs): Preserve OCI publisher routes
- 🐛 (publish): Recheck registry tag collisions
- 🐛 (helm): Validate the packaged chart root
- 🐛 (helm): Reject release-time registry tag collisions
- 📝 (docs): Rename OCI publisher pages
- 📝 (helm): Refine version file guidance
- 📝 (helm): Keep chart guidance together
- 📝 (helm): Clarify the chart archive option
- 💥 (publishers): Rename OCI publisher types
- ♻️ (helm): Reuse OCI publication primitives
- ✨ (helm): Publish packaged charts to OCI registries
- ✅ (publish): Keep collision regression sandboxed

</details>

## [0.8.2] - 2026-11-14

- 🐛 Presswerk evaluates with the user's Nix CLI and enables the experimental features it needs.

<details>
<summary>Commits (4)</summary>

- 🐛 (nix): Enable the required CLI features
- 🐛 (evaluation): Use the user's Nix CLI
- 👷 Pin released binaries in Cachix
- 📝 (docs): Name both required Nix commands

</details>

## [0.8.1] - 2026-11-14

- ⚡️ Building Presswerk from the flake reuses its cached Rust dependencies across releases.

<details>
<summary>Commits (7)</summary>

- 🐛 (build): Reuse cached dependencies for Rust docs
- 🐛 (build): Keep cross-system dependency evaluation build-free
- 🐛 (build): Preserve package outputs with the CLI reference
- ♻️ (build): Build the tests from the shared crate arguments
- ♻️ (build): Generate the CLI reference in postInstall instead of copying crane's install
- ♻️ (build): Explain the version-free dependency build and let Cargo default the version
- ⚡️ (build): Cache Rust dependencies across releases

</details>

## [0.8.0] - 2026-11-14

- ✨ An OCI publisher pushes images built by the flake to container registries, with gzip or zstd
  layers and aliases such as `{major}.{minor}`.
- ✨ `plan` and `publish` check every publisher of the previous release, and a release no longer
  needs a publisher.

<details>
<summary>Commits (54)</summary>

- 📝 (oci): Explain public reads in one paragraph
- 📝 (oci): Introduce the registry examples and move the repository format up
- 📝 (oci): Drop the races paragraph and shorten what aliases count for
- 📝 (oci): Shorten the failed alias paragraph
- 📝 (oci): Introduce the image tag states
- 📝 (oci): Drop the compression retry example
- 📝 (oci): Name Presswerk at the start of the races paragraph
- 📝 (oci): Describe matching from the image tag and set races apart
- 📝 (oci): Say image tag where tag alone is ambiguous
- 📝 (oci): Name tags and the Git tag as the page defines them
- 📝 (oci): Show tag templates next to the tags they render
- 📝 (oci): Introduce tags and aliases in prose
- 📝 (oci): Say plainly what a wrong platform key leaves behind
- 📝 (oci): Tighten the image paragraphs and say why store paths are recorded
- 📝 (publishers): Drop the publisher order and name the local store as a source
- 📝 (publishers): Describe fetching once on the overview
- 📝 (oci): Say why streamed images are buffered and what an image index is
- 📝 (oci): Name both compressions in the module docs
- 📝 (oci): Show tags and a minor alias in the publisher example
- 📝 (oci): Say what streamed images save and what they do not
- 📝 (oci): Say that image executables write an uncompressed archive
- 📝 (docs): Describe publishing on the publishers overview and link code options to a filtered reference
- 📝 (docs): Describe release completion without a required publisher and public OCI reads
- 📝 (docs): Split the release publishers into an overview and one page per publisher
- 🐛 (oci): Tell IPv6 groups from ports and push every Docker Hub spelling to one target
- 🐛 (oci): Accept one spelling per registry and derive its identity alike in Nix and the CLI
- 🐛 (oci): Compare repositories by identity, not spelling
- 🐛 (oci): Bound the archive's JSON files in all and name the supported layout
- 🐛 (oci): Leave hard links to symbolic links unresolved
- 🐛 (oci): Resolve hard links when written and let any entry replace an earlier one
- 🐛 (oci): Compress layers apart from the archive, reject duplicate platforms, and render tags before checking duplicates
- 🐛 (oci): Require schema version 2 of a version-annotated image index
- 🐛 (oci): Accept only an image index as a completed or version-annotated tag
- 🐛 (oci): Report a tag restored by this run as pushed
- ✨ (oci): Choose gzip or zstd layer compression and its level
- ✨ (oci): Push gzipped layers and recognize a release by the store paths of its images
- ✨ (oci): Accept a username that is no secret as username.value
- ✨ (oci): Declare floating tags under aliases, with {major}, {minor}, and {patch}
- ✨ Check every publisher of the previous release and require none
- ✨ (oci): Read public repositories anonymously with pull access only
- ✨ Add an OCI publisher
- 🎨 (docs): End the publishers page with a newline
- 🎨 (docs): Wrap the publisher run order paragraph
- 💄 (docs): Wash sidebar entries on hover and trim the OCI images section
- 💄 Say so when a release plan has no publishers
- ♻️ (oci): Read images only while pushing them, one layer at a time
- ♻️ (oci): Declare images under platforms and run an image only if it is executable
- ⚡️ (oci): Keep small compressed layers in memory and spool large ones to an unnamed file
- ⚡️ (oci): Compress layers while the archive streams instead of unpacking it
- 🥅 (publish): Show why git failed to inspect or push a tag
- 🏷️ (oci): Name the store path annotation dev.presswerk.origin
- 🔒️ (oci): Keep credentials out of repositories and make aliases best effort
- ⏪️ (oci): Compare repositories as declared again and keep only the Docker Hub target fix
- 🦺 (oci): Reject publishers of one repository that share an image tag

</details>

## [0.7.0] - 2026-11-12

- 💥 Every release declares its commit analyzer, such as `analyzer.gitmoji.enable = true`.
- ✨ A Conventional Commits analyzer releases from Conventional Commit subjects.

<details>
<summary>Commits (8)</summary>

- 💄 (docs): Keep the menu button ink on ground in the dark theme
- 💄 Keep breaking Conventional Commits in their type group with a marker
- 📝 (docs): Split the commit analyzers into an overview and one page per analyzer
- 📝 (docs): Document the Conventional Commits analyzer
- ✨ Release reverted Conventional Commits as patches by default
- ✨ Add a Conventional Commits analyzer
- 💥 Require every release to declare its commit analyzer
- ♻️ Select the commit analyzer with an enable option

</details>

## [0.6.2] - 2026-11-12

- 📝 The README and the docs homepage describe Presswerk as reproducible releases.

<details>
<summary>Commits (10)</summary>

- 💄 (docs): Square Starlight's controls and link from the text instead of hero buttons
- 💄 (docs): Lead the hero to Why Presswerk
- 💄 (docs): Put the GitHub action before Get started
- 💄 (docs): Give the hero actions hard edges and an ink outline
- 📝 Describe Presswerk as reproducible releases in the README
- 📝 (docs): Describe Presswerk in llms.txt like on GitHub
- 📝 (docs): Title the homepage after what Presswerk does
- 📝 Sharpen the introduction and add a contributing guide
- 🔥 (docs): Drop the page list from the homepage
- ✅ (docs): Expect the new homepage title in its Markdown copy

</details>

## [0.6.1] - 2026-11-11

- 🐛 The version badge in the docs links correctly again.

<details>
<summary>Commits (3)</summary>

- 🐛 (docs): Put the badge link on one line
- 🐛 (docs): Keep the opening line break out of the badge link
- 🐛 (docs): Keep whitespace out of the badge link

</details>

## [0.6.0] - 2026-11-11

- ✨ GitHub Releases attach build artifacts from the flake.
- 🔥 Presswerk's own releases no longer attach its CLI.

<details>
<summary>Commits (4)</summary>

- 🐛 (release): Substitute artifacts with nix-store instead of realising them as derivations
- 🔥 (release): Stop attaching Presswerk's own CLI
- 📝 (docs): Document GitHub Release assets
- ✨ (release): Attach build artifacts to GitHub Releases

</details>

## [0.5.1] - 2026-11-11

- 📝 The docs read Presswerk's version from the release instead of a version file.

<details>
<summary>Commits (1)</summary>

- 📝 (docs): Read the version from the release instead of a version file

</details>

## [0.5.0] - 2026-11-11

- ✨ Version files can be TOML, JSON, plain text, or any text file matched by a regular
  expression.

<details>
<summary>Commits (2)</summary>

- ✨ (release): Find versions in any text file with a regular expression
- ✨ (release): Read and write versions in TOML, JSON, and plain files

</details>

## [0.4.1] - 2026-11-11

- 🐛 The release commit subject names the declared tag.

<details>
<summary>Commits (2)</summary>

- 🐛 (release): Name the declared tag in the release commit subject
- ♻️ (release): Release Presswerk with its own module and CLI

</details>

## [0.4.0] - 2026-11-11

- ✨ A badge version file generates a version badge for light, dark, or both colour schemes.

<details>
<summary>Commits (5)</summary>

- 📝 (docs): Show the version badge that matches the docs theme
- 📝 (docs): Leave the version badge's link to the README
- 📝 (docs): Show all badges side by side and trim the badge reference
- ✨ (release): Make version badges for light, dark, or both colour schemes
- ✨ (release): Generate a version badge as a version file

</details>

## [0.3.5] - 2026-11-11

- ⚡️ Presswerk's CI deploys the docs from the build it already ran; nothing changes for users.

<details>
<summary>Commits (2)</summary>

- ⚡️ (ci): Link every output in candidate runs instead of building the docs again
- ⚡️ (ci): Deploy the docs from the ci job's build

</details>

## [0.3.4] - 2026-11-11

- 🔍️ Search engines may crawl the docs and find them through the sitemap.

<details>
<summary>Commits (4)</summary>

- 📝 (docs): Give the real reason for the missing llms-full.txt
- 📝 (docs): Explain what agents can read from the docs
- 👷 (ci): Release as the Presswerk-Releases app
- 🔍️ (docs): Allow crawling and point crawlers to the sitemap

</details>

## [0.3.3] - 2026-11-11

- 📝 The README is a short entry point with a minimal release declaration.

<details>
<summary>Commits (5)</summary>

- 📝 Shorten the README example comments
- 📝 Explain the README release example
- 📝 Show a minimal release declaration in the README
- 📝 Trim the README to an entry point
- 🔥 (ci): Remove the Presswerk deploy key setup

</details>

## [0.3.2] - 2026-11-11

- 🌐 The docs are served from presswerk.dev and deployed with each release.

<details>
<summary>Commits (4)</summary>

- ⬆️ (release): Self-release with Presswerk v0.3.1 from the public GitHub URL
- 👷 (ci): Deploy the docs to GitHub Pages from each release
- 🌐 (docs): Serve the docs from presswerk.dev
- 🔧 (release): Release documentation-only changes as patches

</details>

## [0.3.1] - 2026-11-11

- 📝 The published docs are readable by agents through `llms.txt`.

<details>
<summary>Commits (2)</summary>

- 🐛 🐛 (docs): Advertise the llms index from pages
- 📝 📝 (docs): Make published docs agent-ingestible

</details>

## [0.3.0] - 2026-11-10

- 💥 Declarations use the unstable `schemaVersion = 0` until Presswerk 1.0.
- ✨ Gitmoji catalog entries can be removed with `lib.mkForce null`.
- 🚸 `prepare` and `publish` report each step in aligned columns, and release note lines leave out
  the Gitmoji.
- 📝 The documentation site covers the release model, a walkthrough, CI integration, recovery, and
  a generated reference.

<details>
<summary>Commits (124)</summary>

- 📝 📝 (docs): Put the schema stability caution directly under Fields
- 📝 📝 (docs): Keep the schema stability caution as a callout in the field list
- 📝 📝 (docs): Move the schema stability caution into the schemaVersion field
- 📝 📝 (docs): Shorten the declaration schema page and lead its example with the key fields
- 📝 📝 (docs): Shorten the unsupported declaration recovery and drop verify recovery
- 📝 📝 (docs): Explain how reverts are analyzed when recovering a broken candidate
- 📝 📝 (docs): Drop failed preparation from recovery and move unsupported declarations last
- 📝 📝 (docs): Allow cleaning up a tagged release by hand
- 📝 📝 (docs): Shorten the rewrite-history note in recovery
- 📝 📝 (docs): Lead recovery with partial publication
- 📝 📝 (docs): Recover only from failing candidate checks
- 📝 📝 (docs): Count only failing checks as a reason to recover
- 📝 📝 (docs): Explain why and when to recover, and the rewrite-history shortcut
- 📝 📝 (docs): Explain token.secret and link unsupported schemas to recovery
- 📝 📝 (docs): Fix the tag format link on the recovery page
- 📝 📝 (docs): Explain trusting an unsupported predecessor in recovery
- 📝 📝 (docs): Name the actual preparation failures in recovery
- 📝 📝 (docs): Introduce release publishers as the interface to the outside world
- 📝 📝 (docs): Tidy the release publishers page
- 📝 📝 (docs): Drop the spelling note from the default catalog intro
- 📝 📝 (docs): Trim the catalog override section to the example
- 📝 📝 (docs): Show the catalog override as a standalone release module
- 📝 📝 (docs): Stop claiming releases stay on 0.x
- 📝 📝 (docs): State that the step to 1.0.0 must be acknowledged explicitly
- 📝 📝 (docs): Show a full changelog section produced from Gitmoji commits
- 📝 📝 (docs): Stop recommending the unscoped colon style for Gitmoji subjects
- 📝 📝 (docs): Link the Gitmoji specification and show its subject format
- 📝 📝 (docs): Refer to the configurable changelog instead of CHANGELOG.md
- 📝 📝 (docs): State that newer CLIs read older declarations from Presswerk 1.0 on
- 📝 📝 (docs): Recommend the locked CLI instead of requiring it
- 📝 📝 (docs): Fold the commit analyzer and release publisher groups into one page each
- 📝 📝 (docs): Fold the version files group into one page and document that Cargo.lock is checked
- 📝 📝 (docs): Warn that everything reading the version is rebuilt for every release
- 📝 📝 (docs): Explain version conflicts as conflicting module definitions
- 📝 📝 (docs): Say that token.secret names an environment variable and who reads the declaration
- 📝 📝 (release): Document that version is read from the version files and what sourceRoot is
- 📝 📝 (docs): Name the non-forced push as NixCI's guard against stale deploys
- 📝 📝 (docs): Keep only the candidate rerun in the GitHub Actions recovery section
- 📝 📝 (docs): Rebuild the GitHub Actions page around the walkthrough's workflow and its extensions
- 📝 📝 (docs): Phrase the CI requirements as what your CI should do
- 📝 📝 (docs): Describe CI requirements without GitHub's job vocabulary
- 📝 📝 (docs): Say why Presswerk needs no CI integration and how a stale release commit is stopped
- 📝 📝 (docs): Show the verification output in the walkthrough instead of a checklist
- 📝 📝 (docs): Explain the self-dispatch where the walkthrough introduces it
- 📝 📝 (docs): Print build logs in the walkthrough workflow
- 📝 📝 (docs): Build and check everything with one flake check in the walkthrough workflow
- 📝 📝 (docs): Explain where the walkthrough's presswerk CLI comes from
- 📝 📝 (docs): Build without --no-link in the walkthrough
- 📝 📝 (docs): Drop the crate comment from the walkthrough flake
- 📝 📝 (docs): Name the crate separately from the repository in the walkthrough
- 📝 📝 (docs): Show the Cargo.toml version change as a diff and explain the 0.0.0 start
- 📝 📝 (docs): Replace flake-parts with a plain flake in the walkthrough and show real plan output
- 📝 📝 (docs): Use YOUR-REPO as the example repository name
- 📝 📝 (docs): Focus the walkthrough on automatic GitHub releases with a minimal workflow
- 📝 📝 (docs): Trim the GitHub caution and fix small walkthrough inaccuracies
- 📝 📝 (docs): Leave readiness to the reader and drop the GitHub token from the manual example
- 📝 📝 (docs): Tighten the release mode rules and the manual release example
- 📝 📝 (docs): Add overview pages for CI integration, version files, commit analyzers, and release publishers
- 📝 📝 (docs): Put the release mode rules first and name what each step needs
- 📝 📝 (docs): Align the release modes page with the release model wording
- 📝 📝 (docs): Restructure the release model around the release commit lifecycle
- 📝 📝 (docs): Add an adoption stub and mark unwritten pages in the sidebar
- 📝 📝 (docs): Tie the master record to the release commit in the release model
- 📝 📝 (docs): Focus Why Presswerk on the problem, the idea, and its limits
- 📝 📝 (docs): Name the extension groups commit analyzers and release publishers
- 📝 📝 (docs): Rework the overview around one complete release
- 📝 📝 (docs): Make recovery CI-neutral and pin the walkthrough to the documented version
- 📝 📝 (docs): Drop leftover CI assumptions and fix a mis-indented script
- 📝 📝 (docs): Add an untested NixCI integration example
- 📝 📝 (docs): Flag the GitHub requirement as a current limitation
- 📝 📝 (docs): Document automatic and manual releases and stub the extension pages
- 📝 📝 (docs): Match the overview link lists to the analyzer navigation
- 📝 📝 (docs): Sharpen the overview pitch, declaration, and commands
- 📝 📝 (docs): Note that publish skips the token on ordinary commits
- 📝 📝 (docs): Distinguish catalog overrides from new Gitmoji entries
- 📝 📝 (docs): Document that plan and prepare need the GitHub token after the first release
- 📝 📝 (docs): Cross-link the reference from the landing page and the Gitmoji guide
- 📝 📝 (docs): Write the guide
- 📝 📝 (docs): Restore the GitHub Release draft retry guarantee
- 📝 📝 (docs): Document that retries cannot see drafts of interrupted runs
- 📝 📝 (docs): Show an empty filter state and document tag conflicts
- 📝 📝 (docs): Generate the reference from the release module, CLI, and catalog
- 💄 💄 (docs): Mark each release option with an accent bar and align its details
- 💄 💄 (docs): Name the group in previous and next links to build-your-own pages
- 💄 💄 (docs): Name group landing pages after their group in previous and next links
- 💄 💄 (docs): Draw every command after a shell prompt in bold
- 💄 💄 (docs): Draw the workflow's on key as a key, not as a YAML boolean
- 💄 💄 (docs): Link release options in Nix examples to the options reference
- 💄 💄 (docs): Offer a light badge variant and switch the README badge with the colour scheme
- 💄 💄 (docs): Add a Released with Presswerk badge to the brand assets, the README, and a Badges page
- 💄 💄 (docs): Link the GitHub repository from the header
- 💄 💄 (docs): Punch a round spindle hole into the centre of the flake
- 💄 💄 (docs): Quiet the nested sidebar group labels
- 💄 💄 (docs): Fold the guide pages into the Documentation group
- 💄 💄 (docs): Move recovery and the schema under More and soften the current-page marker
- 💄 💄 (docs): Mark links that leave the documentation
- 💄 💄 (docs): List the Gitmoji catalog by release level and square the sidebar bar
- 🐛 🐛 (preparation): Prepare beside unrelated local changes and check out the release files
- 🐛 🐛 (docs): Fit release diagrams on mobile
- 🐛 🐛 (docs): Restore overview navigation on mobile
- 🐛 🐛 (publication): Reject listed GitHub Releases without a tag name
- 🐛 🐛 (docs): Correct the guide examples
- 🐛 🐛 (publication): Find GitHub Release drafts through the paginated release listing
- ✨ ✨ (docs): Replace option tables with filterable option lists on the reference pages
- ✨ ✨ (docs): Make the default Gitmoji catalog filterable
- ✨ ✨ (release): Remove Gitmoji catalog entries with mkForce null
- ✨ ✨ (docs): Link release options in code blocks marked as release modules
- ✨ ✨ (docs): Add a not-found page that leads back to the overview
- ✨ ✨ (cli): Export the command reference from the clap definitions
- 🎨 🎨 (docs): Highlight Nix strings marked with a bash comment as shell
- 🎨 🎨 (docs): Accent numbers, constants and meta features in code
- 🎨 🎨 (docs): Keep unquoted code scalars and keys in ink
- 🎨 🎨 (docs): Use the brand accent for the start badge
- 🚚 🚚 (docs): Put behavior and token before the GitHub Release options
- 🚚 🚚 (docs): Rename the quickstart to walkthrough and drop its badge
- 🚚 🚚 (docs): Merge the Gitmoji guide and catalog into one analyzer page
- 🚚 🚚 (docs): Group analyzer navigation
- 🚸 🚸 (cli): Report each prepare and publish step in aligned columns
- 🚸 🚸 (notes): Leave the Gitmoji out of release note lines
- ✅ ✅ (publication): Prove a malformed release listing stops publishing before a POST
- ✅ ✅ (docs): Check the generated reference pages and their plain-text copies
- 💥 💥 (declaration): Use the unstable schemaVersion 0 until Presswerk 1.0
- 🔒️ 🔒️ (ci): Pass the candidate input to the publish check through the environment
- 📄 📄 (docs): Publish the Gitmoji MIT notice with the generated catalog

</details>

## [0.2.2] - 2026-11-07

- 💄 The docs site and the README carry Presswerk's brand.

<details>
<summary>Commits (10)</summary>

- 💄 💄 (docs): Give the full mark's frame room at the edges
- 💄 💄 (docs): Colour the shell prompt with the accent
- 💄 💄 (docs): Escalate callouts with the accent
- 💄 💄 Put the Presswerk lockup on the README
- 💄 💄 (docs): Theme the docs site
- 🐛 🐛 (docs): Draw brand assets as paths instead of masks
- 🐛 🐛 (docs): Keep the theme picker on narrow splash pages
- 🍱 🍱 (docs): Add brand assets
- 🔨 🔨 (docs): Run the brand asset generator through the flake
- 📝 📝 (docs): Follow the README's locked CLI usage on the landing page

</details>

## [0.2.1] - 2026-11-07

- 📝 The repository is public, with a README and CI that work for outside contributors.

<details>
<summary>Commits (10)</summary>

- 📝 📝 Align public entry points with project style
- 📝 📝 Use the locked CLI in README examples
- 📝 📝 Follow the relocated Gitmoji catalog
- 📝 📝 Add public repository guidance
- 💚 💚 Complete public Cachix workflow gating
- 💚 💚 Restore tokenless Cachix substitution
- 💚 💚 Authenticate read-only release plans
- 👷 👷 Harden CI permissions for public pull requests
- ♻️ ♻️ Restore the base release workflow
- 🔧 🔧 Publish Nix package metadata

</details>

## [0.2.0] - 2026-11-07

- ✨ The Gitmoji catalog is declarable, and commits may use Gitmoji shortcodes.
- 📄 Presswerk is licensed under MIT OR Apache-2.0.

<details>
<summary>Commits (18)</summary>

- 🐛 🐛 (release) Pin self-release across schema boundaries
- 🐛 🐛 (release) Honor declared catalogs across schema changes
- 🐛 🐛 (docs): Keep portable artifact installation
- 🐛 🐛 (docs): Derive package version from release
- 🐛 🐛 (docs): Fix preview environment
- 📝 📝 Clarify schema-crossing integration
- 📝 📝 Clarify pre-1.0 predecessor checks
- 📝 📝 Point to the moved Gitmoji catalog
- 📝 📝 Expand the README for a public repository
- ✨ ✨ (release) Allow explicit Gitmoji shortcodes
- ✨ ✨ (release) Declare the Gitmoji catalog in schema 2
- ✨ ✨ (docs): Scaffold the documentation site
- 🎨 🎨 (hooks) Remove extra spacing
- 👷 👷 Cache CI builds with Cachix
- 🔒️ 🔒️ (hooks) Scan staged changes with Gitleaks
- 📄 📄 License under MIT OR Apache-2.0
- 🧪 🧪 (analysis) Name the test catalog fixture
- ⚡️ ⚡️ (docs): Build site directly into output

</details>

## [0.1.0] - 2026-11-06

- 🎉 Presswerk releases a flake from a Nix declaration: `plan` and `prepare` analyze Gitmoji
  commits and write version files, the changelog, and a release commit, and `publish` creates the
  Git tag and the GitHub Release.

<details>
<summary>Commits (23)</summary>

- ✨ ✨ (analysis) Analyze merges structurally and reverts semantically
- ✨ ✨ Implement plan and prepare (#23)
- ✨ ✨ Publish Git tags and GitHub Releases (#22)
- ✨ ✨ Write version files, changelog sections, and release commits (#20)
- ✨ ✨ Evaluate release declarations at exact commits (#21)
- ✨ ✨ Analyze Gitmoji commits and render release notes (#17)
- ✨ ✨ Classify commits against release tags (#16)
- ✨ ✨ Add the release declaration foundation (#15)
- 🐛 🐛 (analysis) Require a recorded mainline for in-range merge reverts
- 🐛 🐛 (analysis) Read revert identity from the first body line and always file released reverts as rewinds
- 🐛 🐛 Fix release presentation (#24)
- 🐛 🐛 Restore rust-flake native build inputs
- 🐛 🐛 Make Git available to package tests (#18)
- 📝 📝 Define the release declaration module system
- 📝 📝 Embed Nix evaluation through nix-bindings-rust (#13)
- 📝 📝 Settle M1 baseline, release commit notes, and optional Unreleased section
- 📝 📝 Refine the self-release plan
- 👷 👷 (ci) Show the release plan on pull requests
- 👷 👷 Add automatic self-release workflow (#25)
- 🔨 🔨 (hooks) Accept genuine merge commits in the Gitmoji hook
- 🎉 🎉 Add conception documents
- ✅ ✅ (cli) Check --version against the crate version
- 🔧 🔧 Add direnv configuration

</details>
