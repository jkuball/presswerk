# Changelog

## [0.4.1] - 2026-11-11

### 🐛 Fix a bug

- (release): Name the declared tag in the release commit subject

### ♻️ Refactor code

- (release): Release Presswerk with its own module and CLI

## [0.4.0] - 2026-11-11

### 📝 Add or update documentation

- (docs): Show the version badge that matches the docs theme
- (docs): Leave the version badge's link to the README
- (docs): Show all badges side by side and trim the badge reference

### ✨ Introduce new features

- (release): Make version badges for light, dark, or both colour schemes
- (release): Generate a version badge as a version file

## [0.3.5] - 2026-11-11

### ⚡️ Improve performance

- (ci): Link every output in candidate runs instead of building the docs again
- (ci): Deploy the docs from the ci job's build

## [0.3.4] - 2026-11-11

### 📝 Add or update documentation

- (docs): Give the real reason for the missing llms-full.txt
- (docs): Explain what agents can read from the docs

### 👷 Add or update CI build system

- (ci): Release as the Presswerk-Releases app

### 🔍️ Improve SEO

- (docs): Allow crawling and point crawlers to the sitemap

## [0.3.3] - 2026-11-11

### 📝 Add or update documentation

- Shorten the README example comments
- Explain the README release example
- Show a minimal release declaration in the README
- Trim the README to an entry point

### 🔥 Remove code or files

- (ci): Remove the Presswerk deploy key setup

## [0.3.2] - 2026-11-11

### ⬆️ Upgrade dependencies

- (release): Self-release with Presswerk v0.3.1 from the public GitHub URL

### 👷 Add or update CI build system

- (ci): Deploy the docs to GitHub Pages from each release

### 🌐 Internationalization and localization

- (docs): Serve the docs from presswerk.dev

### 🔧 Add or update configuration files

- (release): Release documentation-only changes as patches

## [0.3.1] - 2026-11-11

### 🐛 Fix a bug

- 🐛 (docs): Advertise the llms index from pages

### 📝 Add or update documentation

- 📝 (docs): Make published docs agent-ingestible

## [0.3.0] - 2026-11-10

### 📝 Add or update documentation

- 📝 (docs): Put the schema stability caution directly under Fields
- 📝 (docs): Keep the schema stability caution as a callout in the field list
- 📝 (docs): Move the schema stability caution into the schemaVersion field
- 📝 (docs): Shorten the declaration schema page and lead its example with the key fields
- 📝 (docs): Shorten the unsupported declaration recovery and drop verify recovery
- 📝 (docs): Explain how reverts are analyzed when recovering a broken candidate
- 📝 (docs): Drop failed preparation from recovery and move unsupported declarations last
- 📝 (docs): Allow cleaning up a tagged release by hand
- 📝 (docs): Shorten the rewrite-history note in recovery
- 📝 (docs): Lead recovery with partial publication
- 📝 (docs): Recover only from failing candidate checks
- 📝 (docs): Count only failing checks as a reason to recover
- 📝 (docs): Explain why and when to recover, and the rewrite-history shortcut
- 📝 (docs): Explain token.secret and link unsupported schemas to recovery
- 📝 (docs): Fix the tag format link on the recovery page
- 📝 (docs): Explain trusting an unsupported predecessor in recovery
- 📝 (docs): Name the actual preparation failures in recovery
- 📝 (docs): Introduce release publishers as the interface to the outside world
- 📝 (docs): Tidy the release publishers page
- 📝 (docs): Drop the spelling note from the default catalog intro
- 📝 (docs): Trim the catalog override section to the example
- 📝 (docs): Show the catalog override as a standalone release module
- 📝 (docs): Stop claiming releases stay on 0.x
- 📝 (docs): State that the step to 1.0.0 must be acknowledged explicitly
- 📝 (docs): Show a full changelog section produced from Gitmoji commits
- 📝 (docs): Stop recommending the unscoped colon style for Gitmoji subjects
- 📝 (docs): Link the Gitmoji specification and show its subject format
- 📝 (docs): Refer to the configurable changelog instead of CHANGELOG.md
- 📝 (docs): State that newer CLIs read older declarations from Presswerk 1.0 on
- 📝 (docs): Recommend the locked CLI instead of requiring it
- 📝 (docs): Fold the commit analyzer and release publisher groups into one page each
- 📝 (docs): Fold the version files group into one page and document that Cargo.lock is checked
- 📝 (docs): Warn that everything reading the version is rebuilt for every release
- 📝 (docs): Explain version conflicts as conflicting module definitions
- 📝 (docs): Say that token.secret names an environment variable and who reads the declaration
- 📝 (release): Document that version is read from the version files and what sourceRoot is
- 📝 (docs): Name the non-forced push as NixCI's guard against stale deploys
- 📝 (docs): Keep only the candidate rerun in the GitHub Actions recovery section
- 📝 (docs): Rebuild the GitHub Actions page around the walkthrough's workflow and its extensions
- 📝 (docs): Phrase the CI requirements as what your CI should do
- 📝 (docs): Describe CI requirements without GitHub's job vocabulary
- 📝 (docs): Say why Presswerk needs no CI integration and how a stale release commit is stopped
- 📝 (docs): Show the verification output in the walkthrough instead of a checklist
- 📝 (docs): Explain the self-dispatch where the walkthrough introduces it
- 📝 (docs): Print build logs in the walkthrough workflow
- 📝 (docs): Build and check everything with one flake check in the walkthrough workflow
- 📝 (docs): Explain where the walkthrough's presswerk CLI comes from
- 📝 (docs): Build without --no-link in the walkthrough
- 📝 (docs): Drop the crate comment from the walkthrough flake
- 📝 (docs): Name the crate separately from the repository in the walkthrough
- 📝 (docs): Show the Cargo.toml version change as a diff and explain the 0.0.0 start
- 📝 (docs): Replace flake-parts with a plain flake in the walkthrough and show real plan output
- 📝 (docs): Use YOUR-REPO as the example repository name
- 📝 (docs): Focus the walkthrough on automatic GitHub releases with a minimal workflow
- 📝 (docs): Trim the GitHub caution and fix small walkthrough inaccuracies
- 📝 (docs): Leave readiness to the reader and drop the GitHub token from the manual example
- 📝 (docs): Tighten the release mode rules and the manual release example
- 📝 (docs): Add overview pages for CI integration, version files, commit analyzers, and release publishers
- 📝 (docs): Put the release mode rules first and name what each step needs
- 📝 (docs): Align the release modes page with the release model wording
- 📝 (docs): Restructure the release model around the release commit lifecycle
- 📝 (docs): Add an adoption stub and mark unwritten pages in the sidebar
- 📝 (docs): Tie the master record to the release commit in the release model
- 📝 (docs): Focus Why Presswerk on the problem, the idea, and its limits
- 📝 (docs): Name the extension groups commit analyzers and release publishers
- 📝 (docs): Rework the overview around one complete release
- 📝 (docs): Make recovery CI-neutral and pin the walkthrough to the documented version
- 📝 (docs): Drop leftover CI assumptions and fix a mis-indented script
- 📝 (docs): Add an untested NixCI integration example
- 📝 (docs): Flag the GitHub requirement as a current limitation
- 📝 (docs): Document automatic and manual releases and stub the extension pages
- 📝 (docs): Match the overview link lists to the analyzer navigation
- 📝 (docs): Sharpen the overview pitch, declaration, and commands
- 📝 (docs): Note that publish skips the token on ordinary commits
- 📝 (docs): Distinguish catalog overrides from new Gitmoji entries
- 📝 (docs): Document that plan and prepare need the GitHub token after the first release
- 📝 (docs): Cross-link the reference from the landing page and the Gitmoji guide
- 📝 (docs): Write the guide
- 📝 (docs): Restore the GitHub Release draft retry guarantee
- 📝 (docs): Document that retries cannot see drafts of interrupted runs
- 📝 (docs): Show an empty filter state and document tag conflicts
- 📝 (docs): Generate the reference from the release module, CLI, and catalog

### 💄 Add or update the UI and style files

- 💄 (docs): Mark each release option with an accent bar and align its details
- 💄 (docs): Name the group in previous and next links to build-your-own pages
- 💄 (docs): Name group landing pages after their group in previous and next links
- 💄 (docs): Draw every command after a shell prompt in bold
- 💄 (docs): Draw the workflow's on key as a key, not as a YAML boolean
- 💄 (docs): Link release options in Nix examples to the options reference
- 💄 (docs): Offer a light badge variant and switch the README badge with the colour scheme
- 💄 (docs): Add a Released with Presswerk badge to the brand assets, the README, and a Badges page
- 💄 (docs): Link the GitHub repository from the header
- 💄 (docs): Punch a round spindle hole into the centre of the flake
- 💄 (docs): Quiet the nested sidebar group labels
- 💄 (docs): Fold the guide pages into the Documentation group
- 💄 (docs): Move recovery and the schema under More and soften the current-page marker
- 💄 (docs): Mark links that leave the documentation
- 💄 (docs): List the Gitmoji catalog by release level and square the sidebar bar

### 🐛 Fix a bug

- 🐛 (preparation): Prepare beside unrelated local changes and check out the release files
- 🐛 (docs): Fit release diagrams on mobile
- 🐛 (docs): Restore overview navigation on mobile
- 🐛 (publication): Reject listed GitHub Releases without a tag name
- 🐛 (docs): Correct the guide examples
- 🐛 (publication): Find GitHub Release drafts through the paginated release listing

### ✨ Introduce new features

- ✨ (docs): Replace option tables with filterable option lists on the reference pages
- ✨ (docs): Make the default Gitmoji catalog filterable
- ✨ (release): Remove Gitmoji catalog entries with mkForce null
- ✨ (docs): Link release options in code blocks marked as release modules
- ✨ (docs): Add a not-found page that leads back to the overview
- ✨ (cli): Export the command reference from the clap definitions

### 🎨 Improve structure / format of the code

- 🎨 (docs): Highlight Nix strings marked with a bash comment as shell
- 🎨 (docs): Accent numbers, constants and meta features in code
- 🎨 (docs): Keep unquoted code scalars and keys in ink
- 🎨 (docs): Use the brand accent for the start badge

### 🚚 Move or rename resources (e.g.: files, paths, routes)

- 🚚 (docs): Put behavior and token before the GitHub Release options
- 🚚 (docs): Rename the quickstart to walkthrough and drop its badge
- 🚚 (docs): Merge the Gitmoji guide and catalog into one analyzer page
- 🚚 (docs): Group analyzer navigation

### 🚸 Improve user experience / usability

- 🚸 (cli): Report each prepare and publish step in aligned columns
- 🚸 (notes): Leave the Gitmoji out of release note lines

### ✅ Add, update, or pass tests

- ✅ (publication): Prove a malformed release listing stops publishing before a POST
- ✅ (docs): Check the generated reference pages and their plain-text copies

### 💥 Introduce breaking changes

- 💥 (declaration): Use the unstable schemaVersion 0 until Presswerk 1.0

### 🔒️ Fix security or privacy issues

- 🔒️ (ci): Pass the candidate input to the publish check through the environment

### 📄 Add or update license

- 📄 (docs): Publish the Gitmoji MIT notice with the generated catalog

## [0.2.2] - 2026-11-07

### 💄 Add or update the UI and style files

- 💄 (docs): Give the full mark's frame room at the edges
- 💄 (docs): Colour the shell prompt with the accent
- 💄 (docs): Escalate callouts with the accent
- 💄 Put the Presswerk lockup on the README
- 💄 (docs): Theme the docs site

### 🐛 Fix a bug

- 🐛 (docs): Draw brand assets as paths instead of masks
- 🐛 (docs): Keep the theme picker on narrow splash pages

### 🍱 Add or update assets

- 🍱 (docs): Add brand assets

### 🔨 Add or update development scripts

- 🔨 (docs): Run the brand asset generator through the flake

### 📝 Add or update documentation

- 📝 (docs): Follow the README's locked CLI usage on the landing page

## [0.2.1] - 2026-11-07

### 📝 Add or update documentation

- 📝 Align public entry points with project style
- 📝 Use the locked CLI in README examples
- 📝 Follow the relocated Gitmoji catalog
- 📝 Add public repository guidance

### 💚 Fix CI Build

- 💚 Complete public Cachix workflow gating
- 💚 Restore tokenless Cachix substitution
- 💚 Authenticate read-only release plans

### 👷 Add or update CI build system

- 👷 Harden CI permissions for public pull requests

### ♻️ Refactor code

- ♻️ Restore the base release workflow

### 🔧 Add or update configuration files

- 🔧 Publish Nix package metadata

## [0.2.0] - 2026-11-07

### 🐛 Fix a bug

- 🐛 (release) Pin self-release across schema boundaries
- 🐛 (release) Honor declared catalogs across schema changes
- 🐛 (docs): Keep portable artifact installation
- 🐛 (docs): Derive package version from release
- 🐛 (docs): Fix preview environment

### 📝 Add or update documentation

- 📝 Clarify schema-crossing integration
- 📝 Clarify pre-1.0 predecessor checks
- 📝 Point to the moved Gitmoji catalog
- 📝 Expand the README for a public repository

### ✨ Introduce new features

- ✨ (release) Allow explicit Gitmoji shortcodes
- ✨ (release) Declare the Gitmoji catalog in schema 2
- ✨ (docs): Scaffold the documentation site

### 🎨 Improve structure / format of the code

- 🎨 (hooks) Remove extra spacing

### 👷 Add or update CI build system

- 👷 Cache CI builds with Cachix

### 🔒️ Fix security or privacy issues

- 🔒️ (hooks) Scan staged changes with Gitleaks

### 📄 Add or update license

- 📄 License under MIT OR Apache-2.0

### 🧪 Add a failing test

- 🧪 (analysis) Name the test catalog fixture

### ⚡️ Improve performance

- ⚡️ (docs): Build site directly into output

## [0.1.0] - 2026-11-06

### ✨ Introduce new features

- ✨ (analysis) Analyze merges structurally and reverts semantically
- ✨ Implement plan and prepare (#23)
- ✨ Publish Git tags and GitHub Releases (#22)
- ✨ Write version files, changelog sections, and release commits (#20)
- ✨ Evaluate release declarations at exact commits (#21)
- ✨ Analyze Gitmoji commits and render release notes (#17)
- ✨ Classify commits against release tags (#16)
- ✨ Add the release declaration foundation (#15)

### 🐛 Fix a bug

- 🐛 (analysis) Require a recorded mainline for in-range merge reverts
- 🐛 (analysis) Read revert identity from the first body line and always file released reverts as rewinds
- 🐛 Fix release presentation (#24)
- 🐛 Restore rust-flake native build inputs
- 🐛 Make Git available to package tests (#18)

### 📝 Add or update documentation

- 📝 Define the release declaration module system
- 📝 Embed Nix evaluation through nix-bindings-rust (#13)
- 📝 Settle M1 baseline, release commit notes, and optional Unreleased section
- 📝 Refine the self-release plan

### 👷 Add or update CI build system

- 👷 (ci) Show the release plan on pull requests
- 👷 Add automatic self-release workflow (#25)

### 🔨 Add or update development scripts

- 🔨 (hooks) Accept genuine merge commits in the Gitmoji hook

### 🎉 Begin a project

- 🎉 Add conception documents

### ✅ Add, update, or pass tests

- ✅ (cli) Check --version against the crate version

### 🔧 Add or update configuration files

- 🔧 Add direnv configuration

