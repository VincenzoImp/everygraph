# Golden transcripts of the command

Written by `tests/commands.rs` (`EVERYGRAPH_BLESS=1` writes them again); checked by
`docs/tools/check_transcripts.py`. Times, dates and ages are masked.
Piped: standard output, then standard error.

```console
$ everygraph run components g.eg
node	component
a	0
b	0
c	0
i	0
d	1
e	1
f	1
h	2
$ echo $?
0
```

```console
$ everygraph run pagerank g.eg --top 3
node	pagerank	rank
c	0.2051669186769794	1
e	0.20411998339951568	2
a	0.13758199641304084	3
$ echo $?
0
```

```console
$ everygraph run pagerank g.eg --top 3
node	pagerank	rank
c	0.2051669186769794	1
e	0.20411998339951568	2
a	0.13758199641304084	3
  pagerank                                              kept
$ echo $?
0
```

```console
$ everygraph run kcore g.eg --out core.csv
ok core.csv | 42 B | 8 rows | <time>
$ echo $?
0
```

```console
$ everygraph run bfs g.eg a --top 4
node	distance	parent
a	0
b	1	a
c	1	a
i	2	c
$ echo $?
0
```

```console
$ everygraph run bfs g.eg zz
error: zz is not in g.eg.

  Ways forward:
    check the identifier: it is matched as the input wrote it

Nothing was changed.
$ echo $?
6
```

```console
$ everygraph info g.eg
g.eg | 90.1 kB | created <date> from g.txt

  nodes     8
  edges     6 | undirected | duplicates kept as events | self-loops dropped
  ids       text
  file      generation 4 | 16 sections | 12.3 kB reclaimable

  computed so far
    components  48 B | <time> | <date> | <age>
    kcore       48 B | <time> | <date> | <age>
    pagerank    80 B | <time> | <date> | <age>

  next: everygraph compact g.eg
$ echo $?
0
```

```console
$ everygraph export g.eg --out edges.csv --topology-only
The output is not anonymized: structure alone can identify people.
ok edges.csv | 38 B | 6 edges | <time>
$ echo $?
0
```

```console
$ everygraph compact g.eg
ok g.eg | 77.8 kB | 12.3 kB reclaimed | <time>
$ echo $?
0
```

```console
$ everygraph run pagerank g.eg --max-hops 3
error: Unknown option --max-hops.

  Ways forward:
    everygraph run pagerank --help

Nothing was written.
$ echo $?
2
```
