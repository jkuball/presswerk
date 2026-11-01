# Observability

A release run is a small distributed operation. It evaluates a flake, analyzes history, and
talks to several forges and registries in order, and it may fail halfway and be retried. When
something goes wrong, the questions are always the same: which candidate, which step, which
destination, what remote state did it find, and how long did it take. Presswerk should answer
them through [OpenTelemetry](https://opentelemetry.io/).

Observability is wanted but not a hard requirement for the first release. This document
describes the intended direction.

## Principles

- **Opt-in.** Nothing is exported unless the consumer configures an endpoint. Without
  configuration, Presswerk contacts no telemetry service. This keeps the rule that Presswerk only
  talks to services the consumer configured.
- **Standard configuration.** Exporters are configured through the standard OpenTelemetry
  environment variables, such as `OTEL_EXPORTER_OTLP_ENDPOINT`, so existing collectors and
  runtime setups work without Presswerk-specific settings.
- **Private by default.** Telemetry never contains credentials, and must not contain release
  notes, commit messages, or other repository content unless the consumer explicitly allows it.
  Private repositories must be able to export telemetry to an internal collector without
  leaking content.
- **Same output locally.** Human-readable terminal output and exported telemetry come from the
  same instrumentation, so a local run and a CI run are diagnosed the same way.

## Traces

One release run is one trace. A likely span structure:

```text
presswerk publish
├── evaluate release declaration
├── build candidate context
│   └── commit analysis (analyzer)
├── preflight
│   └── check <publisher>          (one per publisher)
└── publish
    └── publish <publisher>         (one per publisher, in order)
        ├── inspect remote state
        ├── write
        └── verify
```

Useful attributes are the candidate identity (repository, commit, version), the command,
the publisher type and destination, the observed [remote state](glossary.md#remote-state), and
whether the run is a retry. Retries of the same candidate should be recognizable as such,
for example through a stable attribute derived from the candidate identity.

If the runtime passes a trace context, such as a `TRACEPARENT` environment variable, Presswerk
should join it, so a release appears inside the surrounding CI pipeline trace.

## Metrics and logs

Metrics are secondary to traces for a short-lived CLI, but a few are useful across many runs:
run duration and outcome, per-publisher duration and outcome, and counts of remote states found.

Logs should be structured and correlated with the active span, so the terminal output of a
failed run can be matched to its trace.

## Implementation direction

The Rust ecosystem's `tracing` crate as the instrumentation layer, bridged to OpenTelemetry
exporters, is the obvious starting point. Instrumentation should be in place from the start
even if export is added later, because retrofitting spans across adapters is harder than
adding an exporter.

Adapters, including external process adapters, should be able to contribute spans. For process
adapters this means passing the trace context to the child process.

## Open questions

- whether export ships in the first release or later;
- which content, if any, may be exported for public repositories;
- how external adapters report spans and metrics;
- whether metrics are exported at all from a short-lived process, or left to traces.
