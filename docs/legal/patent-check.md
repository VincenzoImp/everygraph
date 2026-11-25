# Freedom to operate: tile-pyramid graph maps

Research date 2026-11-25 (web research by a subagent; not independently re-verified
except where stated). Not legal advice, and not a professional freedom-to-operate search.

## Uncharted Software — the concern that prompted this check

The "patent pending" notice referred to **US 2017/0323028 A1** (application 15/146,407,
"System and method for large scale information processing using data visualization for
multi-scale communities", filed 2016-05-04, inventors Jonker and Langevin). Its claims
describe almost exactly the pipeline planned here: hierarchical communities, layout sized
by node count and spaced by relationship strength, each level assigned to a level of a
tile hierarchy, tiles served to a viewer.

According to Google Patents (<https://patents.google.com/patent/US20170323028A1/en>) it
received a final rejection on 2019-05-21 and was **abandoned on 2019-11-25** for failure
to respond; no continuation and no foreign family members are listed. This was not
confirmed in USPTO Patent Center, CIPO or Espacenet.

Consequences, as facts: an abandoned application grants no rights; the published
application (2017-11-09) and Uncharted's own workshop paper (IEEE VIS, October 2015) are
prior art against later filings by anyone. Uncharted's code is archived: `aperture-tiles`
(MIT, which has no express patent grant) and `salt-core` (Apache-2.0, which has one).

## Granted patents with elements close to the design

| Patent | Owner | What the independent claims require (summarised) | Present in our design? |
|---|---|---|---|
| US 9,299,176 B2 "Visualizing large graphs" (filed 2013-11-19; estimated expiry about 2034) | Microsoft (GraphMaps) | Zoom levels assigned to vertices so that the most important ones fill each region up to a prescribed maximum; zoom levels assigned to "rails" (shared curve segments standing for edges); rendering what intersects the viewport at or below the current zoom | Viewport rendering by zoom level: yes. A per-region quota of nodes ranked by importance: only where tiles pick the top-k entities under a budget (labels, omitted entities). Rails: no — edges are drawn for the selection only. Community hierarchy and aggregates are not part of these claims. |
| US 11,030,249 B2 and continuation US 11,748,412 B2 (provisional 2018-10-01; estimated expiry about 2039) | Palo Alto Networks | A tree of levels of tiles over data points, with a maximum number of points per tile tied to a client resource; a manifest giving the number of points in each tile, sent to the client; tile requests chosen from the manifest | A tile pyramid with an index in one file: yes in kind (PMTiles). Per-tile caps: yes, as byte and entity budgets. A manifest of per-tile point counts sent to the client, levels chosen from client resources, sampling ratios: not in the design. Not specific to graphs. |
| US 8,823,709 B2 (provisional 2007-11-01; estimated expiry about 2031) | eBay | Nodes that are search queries on a host site, clusters of search results, image tiles per zoom level | No: data tiles, and nodes are not search queries. |

Maintenance-fee status of all three is unverified. Also noted, less close: US 12,335,118
(IIIT Hyderabad, spiral layouts), US 10,192,000 (Walmart, query-driven subgraph display).
No graph-tiling patents were found for Graphistry, Kineviz, Linkurious, Cambridge
Intelligence, Palantir, Tableau, Google or IBM, but that was a web search, not a database
search by assignee; several inventor queries failed.

## Prior art before 2016

Multiscale and hierarchical graph views: Auber et al. 2003; ASK-GraphView (Abello, van Ham,
Krishnan 2006, recursive clustering browsed top-down, 16 M edges); TopoLayout, Grouse,
GrouseFlocks (Archambault et al. 2007–2008); Balzer and Deussen 2007. Tiled multi-scale
views: ZAME (Elmqvist et al. 2008); the eBay filing published 2009; GraphMaps (Nachmanson
et al. 2015). Graphs as maps: GMap (Gansner, Hu, Kobourov 2009–2010). Multilevel layout:
Hu 2005, OpenOrd 2011. Nested layouts: circle packing (Wang et al. 2006), treemaps (1991).
Tile pyramids for maps: OSGeo TMS 2006. Uncharted's own code commits 2014–2015.

Recent and very close in spirit, as both competition and prior art: Nachmanson and Chen,
"Browsing Large Graphs with Tile Pyramids and Sleeve Routing in the Browser"
(arXiv 2605.17498, May 2026; MSAGLJS, MIT).

## Verdict for the plan

- **Uncharted: no longer a reason to change the design.** Development proceeds.
- **Before anything is public**, a patent attorney should be asked: whether 15/146,407 has
  any living relative or could be revived; the status and European counterparts of
  US 9,299,176 and US 11,030,249 / 11,748,412; and how "a prescribed maximum number of
  vertices", "rails", "manifest" and "client resource" would be read against our tile
  format. Tracked as a release gate.
- **Design notes carried into the tile format issue:** choose what a tile shows from the
  community hierarchy and exact aggregates rather than from a per-region quota of nodes
  ranked by importance wherever that is equally good; do not introduce shared edge-route
  segments with zoom levels; keep the index a plain tile directory rather than a manifest
  of per-tile point counts negotiated against client resources. These are engineering
  preferences recorded early, not conclusions about infringement.
