# ADR 0011 — Rostering suitability as declared constraints plus derived history

- **Status:** Accepted
- **Date:** 2026-11-07

## Context

Rostering is the point where the care graph becomes actionable: coordinators need
to know which worker is most suitable for a shift. Suitability depends on more
than qualifications:

- repeated, successful work with a particular client;
- familiarity with a site, service or division;
- who else is on the shift (some workers work well together, others do not);
- explicit preferences and restrictions from clients, workers and coordinators.

The existing `relationships` table records semantic care roles — guardian,
general practitioner, employee — and is deliberately hand-curated. It is not a
place to accumulate learned affinity scores. Mixing the two would destroy
auditability, make care relationships look like performance ratings, and make it
impossible to explain why a worker was recommended.

## Decision

Keep **declared relationships** and **observed suitability** as separate layers,
and combine them only when ranking workers.

1. **Declared relationships stay in `relationships`.** They remain the source of
   truth for care and employment roles. Both endpoints carry a role
   (`from_role` / `to_role`).

2. **Explicit rostering constraints live in `roster_constraints`.** A constraint
   targets one worker and one client, worker, site, service or division, with a
   kind of `preferred`, `avoid` or `blocked`. Constraints have a strength, an
   optional reason, a source and effective dates. `blocked` is a hard exclusion;
   `avoid` is a penalty; `preferred` is a bonus. Contact-to-contact constraints
   are treated symmetrically when evaluating either worker.

3. **Shift context is first class.** `sites` records physical locations.
   `shifts` gains `site_id` and `service_id` in addition to `division_id`.
   Suitability is therefore computed against a real site/program/service, not
   just free text.

4. **Assignment outcomes are captured, not inferred.** `worker_assignments`
   keeps its `status` (`assigned`, `cancelled`, `attended`, `no_show`) and gains
   an optional `outcome` (`positive`, `neutral`, `negative`) and a 1–5
   `outcome_rating`. History is append-only from the roster's point of view:
   changing a status or rating changes the evidence, not a mutable score.

5. **History is projected into views, not stored as denormalised edges.**
   `worker_client_history`, `worker_worker_history` and
   `worker_context_history` aggregate attended shifts, no-shows, cancellations,
   ratings, outcomes and recency. Scores are computed from these views on
   demand. This keeps the raw evidence and makes the derivation reproducible.

6. **Recommendations are explainable and advisory.** The suitability endpoint
   returns a score, reasons and warnings for each candidate, plus a separate
   blocked list. It never auto-assigns a worker. Human overrides are allowed and
   should be auditable.

7. **Start with transparent scoring, not machine learning.** Frequency, recency,
   outcomes and explicit constraints are enough to deliver value and can be
   explained to a coordinator. Learned ranking can replace the scoring function
   later without changing the evidence model.

## Consequences

**Positive**

- The care graph remains trustworthy and semantically clean.
- Rostering history accumulates automatically as shifts are assigned and
  completed, so the system improves with use.
- Negative constraints are hard exclusions where needed, not just low scores.
- Every recommendation can be explained with counts, dates and constraint
  reasons.
- The evidence model is future-proof for ML without requiring it.

**Negative**

- More tables and views than a single weighted edge table.
- Scores must be recomputed or cached; there is no single mutable "strength"
  column.
- `contacts` remains untyped, so discovering who is a worker relies on an
  `employee`/`employer` relationship with the organisation or assignment
  history.
- Explicit constraints require human maintenance; they are not learned.

## Alternatives rejected

- **Add `weight`, `polarity` and `confidence` to `relationships`:** conflates
  care roles with performance signals, loses history, and makes "why" opaque.
- **One row per observed worker-client pair with a counter:** simple, but loses
  individual assignment outcomes, recency and auditability.
- **Pure collaborative filtering / ML from day one:** needs far more data,
  produces unexplainable recommendations, and makes negative hard constraints
  hard to enforce safely.
- **Model only negative constraints:** misses the main benefit, which is
  recognising continuity and good outcomes.
