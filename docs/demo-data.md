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
| Clients | 600 (400 NDIS aged 5–64, 200 aged care aged 65+), with address and funding in `details` |
| Staff | 300, with `employee` relationship (and start date) to the organisation |
| Family and emergency contacts | ~1,050 family members plus ~90 neighbours/family friends |
| External organisations | 42 (GPs, a specialist practice, the Office of the Public Advocate, pharmacies, support coordinators, plan managers, allied health, transport, equipment, local businesses, advocacy) |
| External contacts | 2 representatives per organisation, plus 2–4 named GPs per practice and 6 specialists |
| Relationships | ~3,600 two-sided links (see below) |
| Custom fields | `ndis_number`, `plan_end_date` (NDIS clients), `risk_level` (all clients); `employee_number` for staff |
| Custom values | ~1,700 |
| Rostering | 14 days, 420 shifts (~210 completed with outcomes, ~210 published ahead), 84 sessions, 420 client + 462 worker assignments |
| Sites | 4 (day program, respite house, community hub, office), linked to divisions |
| Roster constraints | 5 examples (continuity preference, client avoid, worker-worker block, site/service preference) plus 5 conflict-of-interest blocks |
| Assignment outcomes | Completed shifts carry positive/neutral/negative outcomes and 1–5 ratings |

Every client has at least one relationship. Support networks combine family,
health, coordination and plan-management links.

## People and relationships

Names come from culturally coherent pools (Anglo-Australian, Italian, Greek,
Vietnamese, Chinese, Indian, Arabic, Filipino) that shift with birth year, so a
client born in 1938 reads as "Doreen Fletcher" or "Giuseppe Russo". Every full
name is tracked and clashes are retried, so only a couple of coincidental
duplicates appear across the ~2,200 contacts. Family members share surnames
where they would in real life.

The dataset models the households a disability and aged-care provider sees:

| Pattern | Detail |
|---|---|
| Aged couples | 14 couples who both receive aged care (one same-sex couple), sharing an address and usually a GP |
| Ageing parent carers | 6 aged-care clients still caring for an adult NDIS child at home |
| Siblings | 12 groups of NDIS brothers and sisters, sharing parents |
| SIL houses | 6 supported independent living houses with 3–4 `housemate` residents |
| Cousins | 3 pairs of NDIS cousins |
| Family networks | Parents for younger participants (one marked primary carer), siblings, partners, adult children and grandchildren for older clients, using gendered roles (`mother`/`son`, `husband`/`wife`, …) |
| Decision makers | Parents or siblings as `guardian` (VCAT order), the Office of the Public Advocate for adults with no family, adult children as `enduring_power_of_attorney` |
| Emergency contacts | The first family member is noted as primary emergency contact; clients without family get a neighbour or family friend |
| Health | Each client is a `patient` of a named GP at a practice near home (or of the practice alone, or none recorded); paediatricians for children, geriatricians for older clients, some psychiatry and neurology |
| Funding-specific | NDIS: support coordinator (organisation and named coordinator), plan manager, speech pathology, OT. Aged care: pharmacy, physiotherapy |
| Workers | A few worker couples, siblings, parent/child and cousins |
| Conflicts of interest | 5 workers related to a client, noted on the relationship and blocked by a roster constraint |

## Roster suitability data

The seeder spans two weeks: the previous seven days are completed and attended
with outcome ratings, while the next seven days remain published and assigned.
Some shifts carry a second worker so worker-to-worker history exists, and a
handful of explicit constraints seed the suitability examples (preferred, avoid
and blocked).

## Notes

- Every client is linked to Blue Dreams Care as `care_recipient`/`provider` and
  every worker as `employee`/`employer`. Those relationships are what make a
  contact a client or worker (see `contact_care_roles` in the data model).
- `client_assignments` and `worker_assignments` are attached to a session when
  one exists, otherwise directly to the shift.
- The seeder is a development utility, not part of the product surface.
