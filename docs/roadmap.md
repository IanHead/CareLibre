# Roadmap

CareLibre's product roadmap, derived from the project brief.

**Status:** checked items are implemented and available in the current server
and desktop builds. Unchecked items are under development, planned or under
consideration. Where only part of an area has shipped, the shipped pieces are
checked separately so the remaining work stays visible.

Product work ships in vertical slices: schema, API, tests and documentation
together.

## Features

### Client management

- [x] Client list
- [x] Edit client individual records
- [x] Add, transfer, retire or delete relationships to contact records
  - [ ] Bulk actions on relationships
  - [ ] Sanity checking on relationship actions (e.g. are you really sure you
    want to change this person's father?)
  - [x] Visualise relationships as a constellation in their profile
    (collapsible)
- [x] Wizard-style intake processes
- [ ] Intake, referrals, waitlists, enquiries
  - [x] Guided client, worker and other-contact intake
  - [ ] Referrals
  - [ ] Waitlists
  - [ ] Enquiries
- [ ] Quoting with natural language
- [ ] Assessments and care plans, goal setting, progress tracking
- [ ] Risk assessments, BSPs, Restrictive Practices
- [ ] Case noting
- [ ] Document storage, consent management, service agreements
  - [x] Service agreements — quotes, line items, acceptance and
    finalise-to-roster generation
  - [ ] Document storage
  - [ ] Consent management
- [ ] Customisable document merge templates

### Funding and budget management

- [x] Funding source setups *(server API and admin UI; sources are available
  when creating client packages from the desktop)*
- [ ] Budgets tracked by funding category, plan and quarter (for SAH)
- [ ] NDIS and SAH price lists
  - [ ] Basic support
  - [ ] Built-in, automatic updates
- [ ] Custom price lists and funding types
- [ ] Client contributions and co-payments, care management fees

### Revenue cycle management

- [ ] Timesheets generated from completed shifts, with SCHADS award
  interpretation
- [ ] Payroll export or integration: Employment Hero/KeyPay, Xero, et al.
- [ ] Automatic NDIS rate selection; time of day bands, weekends and public
  holidays, travel, short-notice cancellations
- [ ] Bulk claiming to the NDIA (PACE/PRODA) plus Services Australia claiming
  for Support At Home
- [ ] Invoicing to plan managers and private clients
- [ ] Monthly statements for Support At Home clients
- [ ] Payment reconciliation, handling rejected claims
- [ ] Accounting integrations: Xero, MYOB, QuickBooks, Business Central

### Mobile app for workers

- [ ] Clock on and off with GPS, shift details, client summaries
- [ ] Notes, incident reports, checklists and task lists entered from the field
- [ ] Medication prompts and eMAR in some products
- [ ] Secure messaging and push notifications for shift offers

### Web portal for clients and families

- [ ] Upcoming services, worker details (just a first name and maybe a photo,
  public-facing bio)
- [ ] Messaging with the provider
- [ ] Budget balance and statements
- [ ] Request services changes or cancellations

### Compliance and clinical management

- [ ] Incident management
  - [ ] Basic, including follow-ups and preventative corrections
  - [ ] NDIS reportable incidents
  - [ ] SIRS aged care incident reporting
- [ ] Complaints, compliments and feedback register
  - [ ] Maybe unresolved complaints can factor in to rostering recommendations
    (e.g. family member complained about a worker)
- [ ] Continuous improvement register
- [ ] Risk register
- [ ] Policy acknowledgement
- [ ] Configurable audit trail
  - [ ] Evidence collection for NDIS Practice Standards and the Aged Care
    Quality Standards (strengthened in 2025 under the new Aged Care Act)
- [ ] eMAR/medication management
- [ ] Wound care, vital signs and clinical charting, clinical observations
- [ ] Residential aged care features; AN-ACC assessments, care minutes
  reporting
- [ ] Allied health service records
- [ ] Nursing service records

### Rostering and scheduling

- [ ] Open shifts in sidebar instead of above/below; same with new shift dialog
- [x] Shift and appointment scheduling, recurring rosters, group/1:1 supports
  *(appointments use the shift model; recurring series and multi-person group
  blocks are supported)*
- [ ] Matching workers to clients based on multivariate criteria:
  - [x] Hard compliance requirements: required training, certifications, etc.,
    set globally and per-client; can definitively rule out workers from
    suggestion rather than deranking them
  - [ ] By qualifications, skills, gender, language and client preferences
    - [x] Qualifications, skills, gender and language requirements
    - [ ] Client preference matching
  - [ ] Use a social graph for determining likely fits of workers to clients,
    without requirement of explicitly stating it in the worker's profile (e.g.
    a worker has worked with many clients who prefer an older worker, for
    instance, or recommend a worker who has not worked with this client before,
    but has worked with other clients like them)
  - [x] Negative relationships to actively discourage assigning a particular
    worker (e.g. if they or the client had a bad experience) *(preferred, avoid
    and blocked roster constraints feed suitability; a dedicated management UI
    is still pending)*
  - [ ] Availability, current and ongoing
  - [ ] Award considerations — who's the cheapest worker, award rules
    violations, overtime, minimum breaks, broken shifts, etc.
  - [ ] Criteria biasing configurable via admin
- [ ] Shift bidding and offering
- [x] Copy/paste shifts in rostering

### External communications

- [ ] Configurable external communications (SMS etc.)

### Reporting and analytics

- [ ] Dashboards:
  - [ ] Utilisation
  - [ ] Revenue
  - [ ] Unbilled services
  - [ ] Budget usage
  - [ ] Shift fill rate
  - [ ] Overtime
- [ ] Government reporting — DEX, quarterly financials
- [ ] Report builder — graphical and SQL queries
- [ ] BI exports

## Worker–client affinity design notes

The next matching increment builds on the shipped requirement scoring:

- **Client preferences about workers** — record preferences at intake, for
  example “older worker”, “bubbly/talkative”, “quiet”, “female”, or “same
  cultural background”. These are the client's preferences, not worker traits:
  workers are never asked to declare personality, age or similar attributes.
- **Learned affinity, not declarations** — when a worker works repeated shifts
  with clients who hold a preference, that history becomes evidence. The scorer
  explains it as, for example, “6 attended shifts with clients who prefer a
  bubbly, talkative worker”.
- **Relationship-aware recommendations** — also consider whether the worker has
  successfully supported other clients connected to the current client (family,
  housemates, shared support coordinator, same site or service). This reuses the
  existing relationship graph and assignment history rather than adding a second
  social model.
- **Advisory and explainable** — preference and network evidence adjusts ranking
  and adds reasons or warnings; it never hard-filters unless an explicit block
  is recorded. The coordinator remains the decision-maker.
- **Sensitive preferences** — age, gender, cultural background and personality
  descriptors are sensitive. Require a reason or legal basis, keep the evidence
  aggregate rather than exposing an inferred worker trait, and avoid
  discriminatory hard constraints.
- **Schema sketch** — `client_worker_preferences` (client, dimension, value,
  strength, reason), with derived views such as `worker_preference_affinity` and
  `worker_client_network_affinity`. No worker trait columns.
- **Scoring** — extend `ScoreInput` with `PreferenceEvidence` and
  `NetworkAffinityEvidence`, and surface reasons such as “previously worked with
  three clients connected to this client”.

## Explicitly out of scope for this repo

- Desktop, Android and iOS clients (separate repositories)
- Marketing site and billing infrastructure
