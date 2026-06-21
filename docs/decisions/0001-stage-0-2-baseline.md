# ADR 0001: Stage 0-2 Engine Baseline

## Status

Accepted for the Stage 0-2 implementation baseline.

## Context

The prototype proves the shape of MoFA Engine but overloads one model status field, treats HTTP server code as an SDK, silently skips invalid provider configuration, and does not expose a manual discovery refresh path.

## Decisions

- Keep `/v1/invoke`, `/v1/capabilities`, `/v1/status`, and `/v1/events` compatible while strengthening the internal contracts.
- Add `/v1/discovery/refresh` for manual resource refresh.
- Use canonical model identifiers in the form `provider/model`.
- Keep short model names only when they resolve uniquely.
- Treat named-model fallback as strict by default; callers must opt into named fallback.
- Split backend health, model availability, residency, and execution state.
- Default the daemon bind address to `127.0.0.1`.
- Make production startup use checked configuration and fail on invalid provider kinds or missing configured secrets.
- Keep cloud lifecycle operations as explicit no-ops returning `ModelResidency::Remote`.

## Consequences

- Existing demo clients continue to work with the current request shape.
- The engine can now explain discovery, health, routing, and lifecycle state more precisely.
- Future stages can add memory reservations and richer SDK bindings without changing the basic registry model again.