# ADR 0008 — Configurable permission sets with capability checks

- **Status:** Accepted
- **Date:** 2026-11-07

## Context

The foundation shipped five fixed roles (`owner`, `admin`, `manager`, `staff`,
`viewer`). Real providers do not map onto them: rostering coordinators, intake
teams, HR, finance and house managers need different slices of the same objects,
and the same provider may name or combine those jobs differently. Hard-coding
more roles would produce an ever-growing enum and still miss cases.

The onboarding flow also needs a concrete "set up access" step, which is not
credible while authorisation is a fixed list.

## Decision

1. **Authorisation is capability-based.** Every check is a capability such as
   `rosters.write` or `finance.read`. Role names are not consulted.
2. **Permission sets group capabilities.** Sets are tenant-scoped and editable,
   except Administrator.
3. **Users may hold multiple sets; capabilities are additive (union).**
4. **Seven OOTB personas are seeded per organisation:** Rostering, Intake,
   Human Resources, Senior Management, House Manager, Finance, Administrator.
   They are starting points, not a closed list.
5. **Owner bypass is separate from sets.** The registration account is `is_owner`
   and holds every capability. The last owner cannot be removed, disabled or
   demoted (database trigger).
6. **Administrator is immutable.** It always contains every capability and cannot
   be edited or deleted.
7. **Deny by default.** No sets means no capabilities; a user can authenticate but
   sees nothing.
8. **Server-side enforcement.** UI hiding is cosmetic; handlers call
   `require(capability)`.
9. **Immediate effect.** Capabilities are resolved from the database per request,
   not embedded in the JWT, so permission changes do not wait for token expiry.
10. **Audited.** Set edits and assignment changes write before/after capability
    diffs to `audit_log`.
11. **Database guardrails** where possible: composite tenant keys, `RESTRICT` on
    deleting sets in use, immutable-set trigger, last-owner trigger.

## Consequences

**Positive**

- Providers can model their real jobs without engineering work.
- New features add capabilities, not roles; existing sets can be extended through
  a migration or left alone.
- The onboarding access step becomes concrete and measurable.
- Least privilege becomes practical: House Manager is read-only without a custom
  build.

**Negative**

- More moving parts than an enum: sets, capability catalogue, assignments and
  safety triggers.
- Capability changes are a security-sensitive operation; the UI needs diff
  previews and warnings.
- Every endpoint must remember to check; that discipline has to be enforced in
  review and tests.
- Capabilities are organisation-wide in Phase 1. House Manager being read-only
  org-wide is broader than "their house"; scoped assignments are a follow-up.

## Alternatives rejected

- **Add more fixed roles.** Still misses provider-specific combinations and makes
  every change a migration.
- **Per-user capability checkboxes.** Maximum flexibility, unmaintainable
  administration; sets are the unit people actually reason about.
- **Copy personas on first edit.** Sounds safe but creates drift with no way to
  receive improvements. Editable seeded sets with a stable `persona_key` keep the
  identity while allowing customisation.
- **Capabilities inside the JWT.** Fast but stale; revoking a capability would
  wait up to the token lifetime. Resolve per request and optimise later if
  needed.
