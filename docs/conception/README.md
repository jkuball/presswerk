# Presswerk documentation

Presswerk is at the conception stage. These documents describe intent, not an implementation
contract. Anything marked as illustrative or open may still change completely.

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
- [minijinja](https://github.com/mitsuhiko/minijinja)
- [changelog-merge](https://github.com/cachix/changelog-merge)
- [nix-bindings-rust](https://github.com/cachix/nix-bindings-rust)
- [flake-parts](https://flake.parts/)
- [secretspec](https://secretspec.dev/)
- [OpenTelemetry](https://opentelemetry.io/)
- [Forgejo API usage](https://forgejo.org/docs/latest/user/api/usage/)
