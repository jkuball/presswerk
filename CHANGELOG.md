# Changelog

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

