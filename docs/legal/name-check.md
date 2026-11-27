# Name check: "Everygraph"

Research date 2026-11-25 (web research by a subagent; the key facts below were re-verified
by the coordinator the same day). Not legal advice.

## The finding that matters

**A US trademark application for the identical mark, for the same kind of software, was
filed eight days before this check and one week before the project chose the name.**

| | |
|---|---|
| Mark | EVERYGRAPH (standard characters) |
| Serial | 50223142, USPTO |
| Filed | 2026-11-17 |
| Applicant | an individual in Florida, no attorney of record |
| Class | 9 |
| Goods | "Downloadable computer software platforms for graph database management and analysis." |
| Basis | intent to use (not yet in use in commerce) |
| Status | new application, not yet assigned to an examiner |

Sources, both read directly: <https://tsdr.uspto.gov/statusview/sn50223142> and
<https://www.trademarkia.com/everygraph-50223142>.

What this means in plain terms (to be confirmed by a trademark attorney): an
intent-to-use application, if it is eventually registered, generally gives its owner
priority in the United States from the filing date. The project chose the name on
2026-11-24 and has made no public use of it. Using "Everygraph" for graph-analysis
software in the US would therefore sit under a pending claim by someone else for the
same name and the same goods.

## Other uses of the name

- GitHub organization `everygraph`: taken (created 2025-09-27, display name "Anygraph",
  no public repositories). Verified through the GitHub API.
- GitHub repository `Quanxin-Lee/EveryGraph` (created 2026-10-30): a planned desktop
  application for scientific charts — same broad field (visualization), different product.
- everygraph.com: registered since 2016, parked.
- PyPI `everygraph`, crates.io `everygraph` and `everygraph-core`, npm `everygraph`: free
  (verified). The npm scope `@everygraph` could not be confirmed.
- No company, paper, model or app store product with this name was found by web search.

## Other trademark registers

**TMview** (the search tool of the European IP network, which aggregates the EUIPO, the
national offices of EU states, the UK IPO, WIPO and others) was queried on 2026-11-25
through an automated browser for names containing "everygraph": **one result only, the US
application above.** No EU, UK, Italian or international record under that name appeared.
TMview is not an official register and has no legal effect; close variants ("every
graph", "evergraph") were not queried there. EUIPO eSearch, WIPO Global Brand Database and
the UK IPO's own search were not operated directly.

## Similar names in the field

Omnigraph (an open-source graph database, 2026), AnyGraph (a graph foundation model, and
an iOS app), EverGraph (a dormant diagramming tool; also a watch trademark). None is
identical; all share the "every/any/omni + graph" pattern, which is also descriptive and
therefore weak as a mark.

## Short command names

`eg` is taken (PyPI, crates.io, npm). `evg` is taken on PyPI and npm. `egr`, `evgr` and
`everyg` were free on PyPI, npm and crates.io.

## Assessment and recommendation

Nothing has been published under the name, so changing it now costs a mechanical rename
(repository, crate names, wordmark). Changing it after a public release would cost
adoption. Three options:

1. **Choose another name now**, screening candidates against the trademark registers
   *before* choosing. Recommended.
2. Keep "Everygraph" as an internal working name, publish nothing under it, and decide
   before going public.
3. Keep the name and ask a trademark attorney whether the US application can be opposed
   or whether earlier rights can be shown. The project has no earlier public use.

Candidates screened on 2026-11-25 — free on PyPI, crates.io, npm and as a GitHub
organization, and with **no result in the USPTO index** for the exact word (through
Trademarkia; EU/WIPO/UK not checked): `gigamap`, `vastgraph`, `widegraph`, `graphsphere`.
Also free on the package registries and GitHub, trademark index not yet looked at:
`eachgraph`, `manygraph`, `graphsize`, `graphlode`, `vastmap`, `hugemap`, `graphwide`.

## Owner's decision (2026-11-25)

Told of the US application, the owner asked to **keep "Everygraph"**. The project
proceeds with the name, knowing the risk, and does the following:

1. **Watch the US application** (serial 50223142). It has not been examined; an
   intent-to-use application is registered only after the applicant shows real use in
   commerce. Check its status at each milestone; if it is published for opposition, tell
   the owner at once (the opposition window is short).
2. **Professional opinion, advisable before the first release and not a blocker** (#122;
   `design/DECISIONS.md`, "When the repository becomes public"): whether to use the name
   in the US, whether an EU filing is worthwhile, and its timing — a US applicant can
   claim priority for an EU filing until about 2027-05-17.
3. **Reserve what can be reserved** (issue #10): the package names are held as 0.0.0
   placeholders on crates.io (`everygraph`, `everygraph-cli`), PyPI and npm
   (`everygraph`); domains and handles by the owner (#122).
4. **Keep the rename cheap**: the name appears in the repository name, crate and package
   names, the wordmark and the documentation; nothing else may depend on it.
