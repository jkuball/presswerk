# Changelog

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

