# Release notes

Each release has a hand-written note in this directory, named after its tag. For
example, the `v0.2.0` release uses `v0.2.0.md`. The release workflow checks for
that file and refuses to publish unless it contains `## New`, `## Changed` and
`## Fixed`, in that order.

Write every bullet for someone who uses CareLibre, not someone who builds it. Say
what a care coordinator, worker or administrator can now do, what works
differently, or what no longer goes wrong. Avoid commit hashes, file names,
function names, API details and words such as "refactor" unless the reader would
recognise them.

- **New** — capabilities that did not exist before.
- **Changed** — existing behaviour that works differently.
- **Fixed** — bugs and rough edges that no longer happen.

Keep all three sections. If nothing belongs in a section, write
`- Nothing in this release.` rather than deleting the heading.

Start from [`TEMPLATE.md`](TEMPLATE.md).

## Example

```markdown
## New

- Organisation administrators can now export a contact list as a spreadsheet.
- A worker can hold more than one role without creating a duplicate contact.

## Changed

- Contact search now finds matches in preferred names as well as legal names.
- Signing out now clears the remembered device immediately.

## Fixed

- Creating a second organisation no longer fails when the first one has no sites.
- Long notes no longer stop a contact record from saving.
```
