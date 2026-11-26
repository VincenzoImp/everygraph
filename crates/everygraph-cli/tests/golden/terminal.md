# Golden transcripts of the command

Written by `tests/commands.rs` (`EVERYGRAPH_BLESS=1` writes them again); checked by
`docs/tools/check_transcripts.py`. Times, dates and ages are masked.
On a terminal under `TERM=dumb`: no colour.

```console
$ everygraph run pagerank g.eg --top 3
  pagerank                                              kept
node   pagerank   rank
c         0.205      1
e         0.204      2
a         0.138      3

3 of 8 rows shown
full result: g.eg (cached) · export with --out pagerank.parquet
```

```console
$ everygraph run bfs g.eg a
4 nodes reached · the farthest 2 hops away

node   distance   parent
a             0        -
b             1        a
c             1        a
i             2        c
d             -        -
e             -        -
f             -        -
h             -        -

8 of 8 rows shown
export with --out bfs.parquet
```

```console
$ everygraph info g.eg
g.eg · 111 kB · created <date> from g.txt

  nodes     8
  edges     6 · undirected · duplicates kept as events · self-loops dropped
  ids       text
  file      generation 1 · 22 sections

  computed so far
    components  48 B · <time> · <date> · <age>
    kcore       48 B · <time> · <date> · <age>
    pagerank    80 B · <time> · <date> · <age>
```
