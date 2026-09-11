# Threat Model — flag-kit

Reference: STRIDE. Scope: the crate's public API surface. Trust boundary:
(1) bytes/inputs entering public constructors and parsers, (2) concurrent
callers sharing interior state. flag-kit is an in-process library — it opens
no sockets and inherits the embedding process's trust domain.

Purpose: Feature flags (`flag-kit`) — typed boolean/percentage/user-targeted flags with reload

## Assets

| ID | Asset | Exposed via |
|----|-------|-------------|
| A1 | rollout determinism | hostile input, concurrent callers |
| A2 | flag integrity across reloads | hostile input, concurrent callers |

## STRIDE Analysis

| # | Threat | Category | Surface | Mitigation | Residual risk |
|---|--------|----------|---------|------------|---------------|
| T1 | Flip-flopping assignment across reloads | DoS | `reload` | snapshot swap: evaluation reads a consistent immutable snapshot | documented |
| T2 | Crafted user key biases rollout | Spoofing | `hashing` | key hashing uses a fixed seed documented in code; attackers can compute it (documented residual risk — acceptable for rollout fairness, not security) | documented |

## Repudiation

The crate keeps no audit trail; attribution of calls to callers is out of
scope for an in-process library.

## Out of Scope

- Network transport security (the crate never opens sockets).
- Storage-host compromise: an attacker who controls the host can bypass all
  in-process mitigations.
- Denial of service via resource exhaustion of the host process beyond the
  bounds enforced above.

Reviewed: 2026-09-11
