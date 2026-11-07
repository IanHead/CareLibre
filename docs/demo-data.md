# Demo data — Blue Dreams Care

A repeatable, development-only dataset for demos, screenshots and manual testing.

## Run

```sh
make seed-demo
# or
DATABASE_URL=postgres://... cargo run -p carelibre-server --bin seed-demo -- --confirm
```

The command:

- applies pending migrations;
- **deletes and recreates** the `blue-dreams-care` organisation;
- deletes users with `@bluedreams.example` addresses;
- inserts the dataset in one transaction;
- prints counts and credentials.

It refuses to run without `--confirm` and refuses when `APP_ENV=production`.
It only touches the `blue-dreams-care` tenant; every other organisation is left
alone.

The seed is deterministic (`StdRng` seed `20261107`), so names and structure are
stable across runs; row IDs differ.

## Credentials

All demo users share the password **`BlueDreamsCare!2026`**.

| Email | Access |
|---|---|
| `owner@bluedreams.example` | Owner (bypasses capability checks) |
| `admin@bluedreams.example` | Administrator persona |
| `rostering@bluedreams.example` | Rostering persona |
| `finance@bluedreams.example` | Finance persona |
| `intake@bluedreams.example` | Intake persona |

Organisation slug: `blue-dreams-care`.

## What is populated

| Area | Count / detail |
|---|---|
| Organisation | Blue Dreams Care, Melbourne north, provider |
| Divisions | North, South, East, Respite and Community |
| Cost centres | `CC-100` Programs plus four children; assigned to divisions |
| Services | 12 (NDIS, aged care and private) with funding codes |
| Rates | 2 per service (weekday/weekend) |
| Rate versions | 2 per rate, effective 2026-07-01 and 2027-07-01 |
| Clients | 600 (400 NDIS, 200 aged care), with address and funding in `details` |
| Staff | 300, with `employee` relationship to the organisation |
| Support networks | 600 family/guardian contacts, 200 emergency contacts |
| External organisations | 40 (GPs, pharmacies, support coordinators, plan managers, allied health, transport, equipment, local businesses, advocacy) |
| External contacts | 2 per external organisation (80), linked as representative/organisation |
| Relationships | ~3,220 two-sided links: person_supported/guardian, person_supported/family, participant/support_coordinator, patient/general_practitioner, customer/pharmacy, participant/plan_manager, employee/employer |
| Custom fields | `ndis_number`, `plan_end_date`, `risk_level` for clients; `employee_number` for staff |
| Custom values | ~2,100 |
| Rostering | 14 days, 420 shifts (~210 completed with outcomes, ~210 published ahead), 84 sessions, 420 client + 462 worker assignments |
| Sites | 4 (day program, respite house, community hub, office), linked to divisions |
| Roster constraints | 5 examples: continuity preferences, client avoid, worker-worker block, site/service preference |
| Assignment outcomes | Completed shifts carry positive/neutral/negative outcomes and 1–5 ratings |

Every client has at least one relationship. Support networks combine family,
health, coordination and plan-management links.

## Roster suitability data

The seeder spans two weeks: the previous seven days are completed and attended
with outcome ratings, while the next seven days remain published and assigned.
Some shifts carry a second worker so worker-to-worker history exists, and a
handful of explicit constraints seed the suitability examples (preferred, avoid
and blocked).

## Notes

- Contact role hints live in `contacts.details.contact_role` (`client`, `worker`,
  `dual`, `other`). They are hints for client views, not the source of truth;
  relationships and assignments remain authoritative.
- `client_assignments` and `worker_assignments` are attached to a session when
  one exists, otherwise directly to the shift.
- The seeder is a development utility, not part of the product surface.
