# Governance

How decisions about Everygraph are made today, who holds what, and what changes as more
people join. This describes the project as it is, not as it hopes to be.

## Today: one owner, development coordinated by an AI agent

The project has **one owner and maintainer**, Vincenzo Imperati (GitHub `VincenzoImp`).
The owner holds the repository, the name, and every account and credential of the
project, and is the person legally responsible for what the project publishes.

On 2026-11-24 the owner delegated the project's decisions and their execution to
**Claude, an AI agent made by Anthropic**, which coordinates development under the rules
in [CLAUDE.md](CLAUDE.md). In practice:

- The coordinating agent plans the work, decides, and assigns each issue to another agent
  instance that implements it on its own branch and opens a pull request. The
  coordinator reviews and merges. Agents' commits are made under the owner's GitHub
  account; there is no separate bot account.
- What needs a person stays with the owner: logins, credentials and registry tokens,
  payments, signatures, messages sent in the owner's name, and professional legal advice
  ([CLAUDE.md § 8](CLAUDE.md)).
- Some steps are irreversible — making the repository public, the first release on a
  package registry, any public announcement. They are written into `docs/STATE.md` and the
  session report first and carried out in a later session, so the owner has seen them.
  Tooling settings in `.claude/settings.json` deny publishing, visibility changes,
  force-pushes and repository deletion until the owner lifts them.

The repository is public since 2026-11-26, during development and before the first
release (`design/DECISIONS.md`, "When the repository becomes public"). Contributions
follow [CONTRIBUTING.md](CONTRIBUTING.md).

## How decisions are made and recorded

- **Architecture and product.** [docs/DESIGN.md](docs/DESIGN.md) lists the decisions with
  the evidence behind each. A decision changes only through an architecture decision record
  under `docs/adr/` (context, measurement, decision, consequences), with `DESIGN.md`
  updated in the same pull request and the change noted in `docs/STATE.md`
  ([CLAUDE.md § 5](CLAUDE.md)). The three product promises in `DESIGN.md` § 2 change
  only with evidence and such a record.
- **What users see and type** (viewer, API names, commands, visual identity) is built to
  the previews under `design/preview/`; the choices made on them are recorded with their
  date in [design/DECISIONS.md](design/DECISIONS.md).
- **The work** is the set of issue files under `plan/issues/`, published as GitHub
  issues. New ideas become issue files before they become code.
- **Changes** reach `main` through pull requests, one per issue, merged when the checks
  pass and the review required by [CLAUDE.md § 4](CLAUDE.md) has no open findings.
  Rules for contributors are in [CONTRIBUTING.md](CONTRIBUTING.md).
- **Conduct** follows [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md). Its "community leaders"
  are, today, the owner.

## Licences and contributions

Code is licensed Apache-2.0 OR MIT; documentation and format specifications CC BY 4.0
([LICENSE-DOCS](LICENSE-DOCS)). Contributors keep their copyright and license their
contribution under the same terms, certified by a Developer Certificate of Origin
sign-off; there is no contributor licence agreement.

Two commitments follow, and are made here so they can be held against the project:

- **The format specifications are public and versioned.** Every on-disk and map format
  the engine writes is specified in the repository under CC BY 4.0, with a version number
  in the format itself, so that other software can read it without Everygraph.
- **No feature is held back for a paid edition.** Whatever the project builds is released
  under the licences above.

Because there is no licence agreement transferring rights, the licence of existing code
cannot be changed to a more restrictive one without the agreement of everyone who holds
copyright in it.

## The name

Rights in the name "Everygraph" — and any trademark registration the project applies for
— are held by the owner, to be transferred to a neutral home if the project gets one
(below). No trademark is registered today, and a third party has a pending US
application for the same name ([docs/legal/name-check.md](docs/legal/name-check.md));
the owner accepted that risk when the repository became public, and professional advice
is planned before the first release (issue #122).

## When there are more maintainers

None of this applies yet; it is the plan for when people join.

- **Contributor**: anyone whose pull request has been merged.
- **Reviewer**: a contributor who has made several substantial contributions to an area
  and is invited by the maintainers to review pull requests there. Gets triage rights.
- **Maintainer**: a reviewer who has worked in the project for some months, invited by
  agreement of the existing maintainers. Gets write access, merges pull requests, and
  takes part in decisions.
- **Release rights** — the credentials to publish to crates.io, PyPI and npm and to tag a
  release — are held by **at least two people before 0.1** is released, so that the
  project can ship a fix if one of them is unavailable.
- **Decisions** among maintainers are by consensus in the pull request or issue concerned.
  Where there is none, the owner decides until the project has a neutral home.
- A maintainer who has been inactive for a year becomes emeritus, and their access is
  removed; they can return by asking.

This document then gets a list of maintainers and of who holds release rights.

## A neutral home

If the project gains users and contributors beyond its owner, the aim is a neutral home
that holds the name and the accounts and outlives any single person or company: a fiscal
sponsor such as NumFOCUS, the OpenJS Foundation for the viewer, or a small foundation of
its own. Funding, partnerships and the choice among these are the subject of issue #115.

## Bus factor

Today the bus factor is one: if the owner is unavailable, nobody else can merge, publish
or administer the repository. What reduces the loss meanwhile is that the project's state
lives in the repository rather than in anyone's head — decisions with their evidence,
the full plan as issue files, `docs/STATE.md`, and the scripts that reproduce the lab's
measurements. The remedy — a second person with administrator and release rights before
0.1 — is planned in issue #115 and is a condition of that release above.

## Changing this document

Through a pull request, like any other change. A change to the commitments under
"Licences and contributions" is a decision change and needs a decision record.
