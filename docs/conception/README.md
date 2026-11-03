# Presswerk documentation

Presswerk is at the conception stage. These documents describe intent, not an implementation
contract. The [self-release milestone](roadmap.md#self-release-milestone) sets the current
scope; many examples describe later functionality. GitHub issues and milestones will track
implementation, while these conception files remain useful local design references.
Illustrative or open details may still change.

| Document | Contents |
| --- | --- |
| [Overview](overview.md) | Motivation, purpose, goals, non-goals, and target users |
| [Product shape](product-shape.md) | Rust CLI, release declaration in the flake, and the flake-parts module |
| [Release model](release-model.md) | Candidate, release context, versions, branches, commit analysis, and changelogs |
| [Scenarios](scenarios.md) | Example releases as Git graphs and terminal sessions |
| [Declaration scenarios](declaration-scenarios.md) | The same release declared as plain data, library function, and flake-parts module |
| [Publication](publication.md) | Publishers, forges, security, and recovery |
| [Runtime integration](runtime-integration.md) | Local shells, CI systems, and secrets |
| [Observability](observability.md) | Opt-in OpenTelemetry tracing, metrics, and logs |
| [Roadmap](roadmap.md) | First consumers and the self-release milestone |
| [Open decisions](open-decisions.md) | Everything that is deliberately undecided |
| [Glossary](glossary.md) | Names and analogies used across the project |

## References

- [semantic-release](https://github.com/semantic-release/semantic-release)
- [Python Semantic Release](https://python-semantic-release.readthedocs.io/en/latest/)
- [git-cliff](https://git-cliff.org/)
- [Gitmoji specification](https://gitmoji.dev/specification) and
  [Gitmoji catalog](https://github.com/carloscuesta/gitmoji/blob/master/packages/gitmojis/src/gitmojis.json)
- [rust-flake single-crate example](https://github.com/juspay/rust-flake/blob/main/examples/single-crate/flake.nix)
- [minijinja](https://github.com/mitsuhiko/minijinja)
- [changelog-merge](https://github.com/cachix/changelog-merge)
- [nix-bindings-rust](https://github.com/nixops4/nix-bindings-rust)
- [flake-parts](https://flake.parts/)
- [secretspec](https://secretspec.dev/)
- [OpenTelemetry](https://opentelemetry.io/)
- [Forgejo API usage](https://forgejo.org/docs/latest/user/api/usage/)
