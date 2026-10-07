# Exploring a note's relations (not started -- design record only)

Not picked up yet. This file exists so a future session can resume the
design conversation without re-deriving it. Nothing below has been
implemented.

## Where this comes from

Originally raised as the third `ViewSwitcher` slot (alongside Body and
Properties, see `ui/components.slint`'s `ViewSwitcher`/`NavPill`):
"本文、プロパティ、関係グラフ" was the user's own list of views a note
sheet would eventually need. This file is that third view's design.

The goal: given one open note, explore the notes related to it. Not a
whole-vault atlas -- scoped to one note's neighborhood, recentered each
time a different note opens.

## Anti-thesis: why not a conventional link graph

The user's own critique of typical graph views (force-directed nodes,
pan/zoom canvas), stated directly:

1. **Not actually good for exploration.** A graph only conveys link
   count, reference direction, and note titles -- a plain list conveys
   at least as much, plus lets you sort/group. Nodes scatter around the
   target note essentially at random (physics-simulated layout, not
   meaningful placement), which doesn't aid exploration or new
   insight into relationships.
2. **Mismatched for mobile.** Pan/zoom is the right interaction model
   for vast, dense, regular data (street maps) that genuinely needs
   incremental zooming to take in. A sparse link structure doesn't
   have that density, so the same interaction model just adds
   sluggishness/lag without a matching payoff.

## Direction 1 (recommended first step): ditch the canvas, use a list

Same vertical-scroll pattern as `PropertiesView` -- no custom
rendering, reuses the app's existing visual language and is cheap to
build:

- Sections: "Links from this note" / "Links to this note" (backlinks).
- Each row: title + snippet + (if available) a small badge for how
  many of its own relations it has.
- Tapping a row expands it **in place** (accordion), indented, showing
  *that* note's own relations -- this is the 2-hop mechanism. Indentation
  preserves the path (which note led to which), which a flat "2-hop
  notes" section would lose -- that loss is exactly the graph's own
  "only conveys title/count/direction" weakness repeated in list form.
- Open question not yet settled: unlimited recursive expansion at any
  depth, vs. a fixed 1-hop/2-hop toggle that caps how far expansion can
  go. Leaning toward a toggle (simpler, avoids runaway nesting), but
  not decided.

## The "can only see the first ~10" problem

Observed directly from the user's own usage: regardless of sort order,
only the first ~10 or so entries actually get looked at. Alphabetical
and last-modified-time orderings are both irrelevant to actual
relevance, so a static list order doesn't help discovery/exploration
much -- the user wanted "ordered by content" but assumed that requires
heavy ML (embeddings) and is out of reach.

### Lightweight relevance signals (no ML/embeddings needed)

Four ideas, all computable from data the app already has or can derive
cheaply:

1. **Common neighbors** (graph-structural): notes that share other
   linked notes with the target are more central to its cluster --
   classic link-prediction heuristic, pure graph math, no NLP.
2. **Lexical overlap** (lightweight "content" signal): word/phrase
   overlap between the target note's text and a candidate's, e.g.
   TF-style scoring -- distinguishes same-topic from unrelated notes
   without real semantic understanding. Worth checking whether
   `immermemo-index`'s existing search-query scoring machinery can be
   reused directly here (treat the target note's own body as the
   "query") -- not yet verified.
3. **Link-edit recency, not note-mtime**: when was *this specific
   link* added/touched (via git history), rather than the whole note's
   last-modified time -- distinguishes an active train of thought from
   a link added years ago and never revisited.
4. **Navigation history reuse**: `session.rs`'s `BackForwardStack`
   (already built, for the note-sheet `NavPill`) is itself a free,
   personalized relevance signal -- notes the user actually jumps to
   together from here, with zero new instrumentation needed.

These four are valuable independent of which view (list or map, below)
ends up consuming them -- worth building regardless of the UI direction
decision.

## Direction 2 (the user's own idea): a static honeycomb map

Raised independently, previously prototyped in part in another of the
user's own projects ("mumeum"). Not a force-directed graph -- a
deterministically generated, regular (hex-grid) layout:

- **Position = relevance.** Distance from the center (target note)
  encodes the combined relevance score (the four signals above) --
  closer rings are more related, same idea as the list's ordering
  problem, just spatial instead of linear.
- **Areas = clusters.** Notes aren't scattered individually; notes
  sharing a tag/kind, or belonging to the same graph-community, are
  placed in contiguous hex regions -- a visually readable "this whole
  neighborhood is about topic X."
- **Roads = explicit links.** Distance-based placement alone loses the
  graph's one real piece of hard information (an actual, specific
  link) the moment two linked notes land far apart spatially. Roads
  restore that as an explicit overlay: a drawn path between any two
  directly-linked notes, independent of their spatial distance.
- **Static, not physics-simulated.** Computed once per
  open-note-recenter, rendered as plain tiles -- this is what actually
  reconciles anti-thesis #2: pan/zoom becomes the *right* interaction
  model once the underlying data is actually regular/tiled, unlike a
  live force-directed sim.

### Why this is hard (acknowledged directly by the user)

Not the rendering -- that's "hard but visible" work (hex tiles, roads,
pan/zoom camera in Slint, all buildable). The real risk is the
**placement algorithm**: "distance = relevance" and "area = cluster
contiguity" are two constraints that can conflict (a high-relevance
note from cluster A may have no free near-center hex because cluster
B's members already claimed that ring) -- a genuine constraint-
satisfaction/packing problem to design from scratch, unproven until
something is actually built and looked at.

## Agreed staging (not started)

1. Build the four relevance signals first -- shared value regardless of
   which view direction is chosen later.
2. Ship Direction 1 (list/accordion) first -- low risk, reuses existing
   patterns, addresses both anti-theses on its own.
3. Treat Direction 2 (honeycomb map) as a separate, later spike:
   validate the placement algorithm cheaply (a throwaway script, not
   production Slint code) before committing to building the renderer.
