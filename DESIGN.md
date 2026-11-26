---
name: lanlord
description: A live departures board for one LAN segment.
colors:
  signal-amber: "#eda23a"
  signal-amber-day: "#875000"
  enamel-night: "#15161a"
  enamel-day: "#eceef0"
  flap-ivory: "#e9e5da"
  flap-night: "#191b1e"
  ink-night: "#e9e5da"
  ink-day: "#191b1e"
  dim-night: "#9c968a"
  dim-day: "#5f656c"
  rule-night: "#33353b"
  rule-day: "#d2d6da"
  rule-strong-night: "#4d5057"
  rule-strong-day: "#adb3b9"
typography:
  figures:
    fontFamily: "system-ui, -apple-system, 'Segoe UI', Roboto, Helvetica, Arial, sans-serif"
    fontSize: "40px"
    fontWeight: 700
    lineHeight: 1
    letterSpacing: "-0.02em"
  wordmark:
    fontFamily: "system-ui, -apple-system, 'Segoe UI', Roboto, Helvetica, Arial, sans-serif"
    fontSize: "20px"
    fontWeight: 700
    letterSpacing: "0.34em"
  device:
    fontFamily: "system-ui, -apple-system, 'Segoe UI', Roboto, Helvetica, Arial, sans-serif"
    fontSize: "15px"
    fontWeight: 600
    lineHeight: 1.45
  body:
    fontFamily: "system-ui, -apple-system, 'Segoe UI', Roboto, Helvetica, Arial, sans-serif"
    fontSize: "15px"
    fontWeight: 400
    lineHeight: 1.45
  label:
    fontFamily: "ui-monospace, 'SF Mono', Menlo, 'Cascadia Mono', 'DejaVu Sans Mono', monospace"
    fontSize: "11px"
    fontWeight: 500
    letterSpacing: "0.12em"
  data:
    fontFamily: "ui-monospace, 'SF Mono', Menlo, 'Cascadia Mono', 'DejaVu Sans Mono', monospace"
    fontSize: "13px"
    fontWeight: 400
    letterSpacing: "0.03em"
rounded:
  square: "0px"
spacing:
  sm: "8px"
  md: "14px"
  lg: "20px"
  xl: "28px"
components:
  filter-key:
    backgroundColor: "transparent"
    textColor: "{colors.ink-night}"
    typography: "{typography.label}"
    padding: "7px 16px"
  filter-key-current:
    backgroundColor: "{colors.flap-ivory}"
    textColor: "{colors.enamel-night}"
    typography: "{typography.label}"
    padding: "7px 16px"
  state-flap:
    backgroundColor: "{colors.flap-ivory}"
    textColor: "{colors.enamel-night}"
    typography: "{typography.label}"
    padding: "4px 10px"
  state-flap-departed:
    backgroundColor: "transparent"
    textColor: "{colors.dim-night}"
    typography: "{typography.label}"
    padding: "4px 10px"
  arrival-flag:
    backgroundColor: "transparent"
    textColor: "{colors.signal-amber}"
    typography: "{typography.label}"
    padding: "1px 6px"
  register-row:
    backgroundColor: "transparent"
    textColor: "{colors.ink-night}"
    padding: "13px 0"
  board-shell:
    backgroundColor: "{colors.enamel-night}"
    textColor: "{colors.ink-night}"
    padding: "26px 28px 40px"
    width: "1040px"
---

# Design System: lanlord

## Overview

**Creative North Star: "The Segment Departure Board"**

lanlord is a split-flap departures board for one LAN segment. The page is an
enamel panel, near-black at night and cool pale enamel in daylight, ruled by
hairlines and carrying exactly one signal colour. The register is a timetable:
each row opens with when the device was last seen, names the device in
sentence case, and closes with a state flap. Nothing is rounded, nothing
floats, and nothing is decorated that is not a fact from the last scan.

The system is built for the operator's brief visit, so hierarchy is temporal
before it is alphabetical. Present devices stack at NOW, departures keep their
place below a structural rule, and the stopwatch at top right states how often
the board refreshes. A device that arrived in the last 24 hours carries a
1px amber arrival flag; that and the refresh clock are the only places amber
is allowed. The result should feel like infrastructure that was installed, not
a dashboard that was assembled.

**Key Characteristics:**
- Enamel ground, ivory flap cells, one amber signal.
- Zero radii; 1px rules do all the separating.
- Last-seen time leads every row; state closes it.
- Split-flap cells for presence, never coloured dots.
- Identical light and dark boards, chosen by the system appearance.

## Colors

A two-temperature enamel palette with a single warm signal.

### Primary

- **Signal Amber** (#eda23a night / #875000 day): the refresh clock at top
  right, the 24-hour arrival count, the arrival flag, and focus outlines.
  It is the only saturated colour on the page.

### Neutral

- **Enamel Night** (#15161a): the board ground at night. Flat, slightly cool,
  with no gradient or glow.
- **Enamel Day** (#eceef0): the same board in daylight. Cool pale enamel, not
  cream; the dark cells invert into near-black flaps.
- **Flap Ivory** (#e9e5da): present state cells and the selected filter key at
  night. It is the warm counterweight to the cool ground.
- **Flap Night** (#191b1e): the same cells and the selected key in daylight.
- **Ink** (#e9e5da night / #191b1e day): device names and body copy.
- **Dim Ink** (#9c968a night / #5f656c day): labels, addresses, MACs, departed
  rows, and the colophon.
- **Rule** (#33353b / #d2d6da) and **Rule Strong** (#4d5057 / #adb3b9):
  hairline row separators and structural dividers.

### Named Rules

**The Two-Use Amber Rule.** Amber belongs to the refresh clock and the
arrival signal. Focus outlines may borrow it as an accessibility state;
nothing else may. Its rarity is how the operator finds "new" in one pass.

**The Enamel-Not-Cream Rule.** Daylight ground is cool pale enamel
(#eceef0), never warm beige. Warmth lives in the flap cells, not the field.

## Typography

**Display Font:** system UI sans (system-ui, Segoe UI, Roboto, Helvetica,
Arial)
**Body Font:** system UI sans
**Label/Mono Font:** system monospace (SF Mono, Menlo, Cascadia Mono, DejaVu
Sans Mono)

**Character:** Two voices, both already on the machine. The sans carries names
and counts in sentence case; the monospace carries timestamps, addresses, MACs
and structural labels. This is a self-hosted utility with no external asset
requests, so the system stacks are the deliberate choice, not a fallback.

### Hierarchy

- **Figures** (700, 40px, 0.9 line-height, -0.02em): the four counts
  (known, present, departed, arrivals). Tabular numerals so they do not jitter.
- **Wordmark** (700, 20px, 0.34em, uppercase): `LANLORD` only.
- **Device name** (600, 15px): sentence case as recorded; no-name devices are
  italic 400 in Dim Ink.
- **Body** (400, 15px, 1.45): empty-state copy and explanations; measure
  capped near 52ch.
- **Segment label** (500, 12px, 0.03em, mono): the segment line under the
  wordmark in the masthead.
- **Clock value** (700, 26px, 1 line-height, mono): the refresh poll interval
  at top right; the only 26px size on the board.
- **Label** (500, 11px, 0.12em, uppercase, mono): figure labels, table
  headers, filter keys, flaps, divider labels, clock caption.
- **Data** (400, 13px, 0.03em, mono, tabular): last-seen age, first-seen age,
  address; MAC at 11px beneath the name.
- **Empty-state body** (400, 14px, 1.45, sans): body copy in the empty state,
  one size above the 11px label floor.

### Named Rules

**The Sentence-Case Data Rule.** Device names are data and stay in sentence
case. Uppercase is a structural voice for labels, figures, flaps and the
wordmark only.

**The Terse Age Rule.** Last seen reads `NOW`, `10 M`, `2 H`, `21 D`. A
present device never reads "0 minutes ago".

## Layout

One 1040px board centred on the enamel field, with 26–28px gutters at desktop
and 16px at phone width. The masthead is a two-end flex row: wordmark and
segment line left, clock right. Below it, a four-cell figure strip is divided
by hairlines, then the filter key row, then the scan-status strip, then the
register.

The register is a table with a deliberate reading order: Last seen · Device ·
Address · First seen · State. Address and First seen are right-aligned data
columns; Device gets the flexible remainder so names never hyphen-wrap
prematurely. The departed group is separated by a full-width divider row
rather than a separate page or filter. At ≤720px the figure strip becomes a
2×2 grid with figures at 30px, the Address column collapses into the device
cell, First seen is hidden, and the state column keeps its place at the right edge.

## Elevation & Depth

There are no shadows and no floating layers. Depth is not conveyed at all;
the page is one flat enamel panel. The only edge treatment is a 1px inset
hairline around the board (`inset 0 0 0 1px`, #2a2c31 night / #dde1e4 day),
which reads as the panel's thickness rather than a lifted card. Separation
comes from tonal rules and the white/dark inversion of flap cells.

## Shapes

Everything is square. Radii are zero on keys, flaps, flags, cells and rows.
The recurring silhouette is the split-flap cell: a rectangular block with a
1px horizontal seam at its midpoint, produced by a two-stop linear gradient.
Departed flaps drop the fill and keep a 1px rule, so absence is drawn as an
outline rather than a second colour.

## Components

### Filter keys

- **Shape:** a single 1px-ruled box, square corners, keys divided by 1px
  vertical rules.
- **Default:** transparent on enamel, Ink label, 11px mono caps.
- **Current:** inverted into a flap cell (ivory on night, near-black on day),
  marked `aria-current="page"`.
- **Hover / Focus:** hover tints the row; focus draws a 2px amber outline
  inset so the key's footprint does not move.

### Figure strip

- **Shape:** four equal cells separated by 1px vertical rules; the strip is
  closed by a 1px rule below.
- **Content:** 40px tabular numeral over an 11px mono caps label. The arrival
  cell and its label are the only amber pair.
- **State:** values are live; the strip is swapped out-of-band on every poll so
  filters keep keyboard focus.

### State flap

- **Present:** ivory flap cell (near-black in daylight) with a horizontal seam
  and Ink caps reading `Present`.
- **Departed:** no fill, 1px Rule Strong border, Dim Ink caps reading
  `Departed`. Never a green/red dot and never colour alone.

### Register row

- **Shape:** table row with a 1px bottom rule; 13px vertical padding; no
  zebra striping and no card wrapping.
- **Reading order:** tabular age, device name with MAC beneath, address,
  first-seen age, state flap.
- **Hover:** a 3.5% ink tint across the row. Departed rows drop name and data
  to Dim Ink at weight 500.
- **Arrival:** a 1px amber `arrival` flag sits inline after the name.

### Refresh clock

- **Shape:** right-aligned in the masthead: 26px mono value (`10 s`) over an
  11px caps caption (`between refreshes`).
- **Motion:** a 1px Dim Ink sweep under the scan-status strip runs once per
  poll interval, linear, and stops under reduced motion. It is a cadence
  mark, not an accent.

## Do's and Don'ts

### Do:

- **Do** lead every register row with its last-seen age and close it with a
  state flap.
- **Do** keep the departed group on the page under a structural divider; an
  empty slice explains itself and links back to the full register.
- **Do** keep amber to the refresh clock and arrival signal; use focus amber
  only for keyboard state.
- **Do** use tabular numerals for every time, count and address.
- **Do** hold the 11px floor for functional labels and the 4.5:1 contrast
  floor in both appearances.
- **Do** treat daylight as an equal board: cool enamel ground, inverted flap
  cells, same amber signal.

### Don't:

- **Don't** round corners, add shadows, or float panels on the board.
- **Don't** use cards, coloured status dots, or a separate "departed" page.
- **Don't** uppercase device names; they are data, not labels.
- **Don't** render a zero-duration age as "0 minutes ago"; use `NOW`.
- **Don't** add a kicker or eyebrow above a heading, or any ornamental label
  that does not state a scan fact.
- **Don't** load external fonts, icons or CDN assets; HTMX is vendored and the
  type is on the machine.
