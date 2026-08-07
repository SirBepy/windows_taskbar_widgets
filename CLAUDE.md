# windows_taskbar_widgets

## Fixed-size rule

Once a widget tile or flyout overlay is mounted, its footprint must not change size: not on
hover, not on content change. A widget whose content length can vary (a process list, a drive
list) must size for its MAX case at declare time, not resize live. Overflow beyond that max
scrolls inside the fixed box (`#flyout`'s `overflow-y: auto`), it never grows the window.

Distinction: a live value change (a stat updating, data arriving) must never resize. A config
toggle (`show_temp`, `show_percent`, `tile_drive`) MAY resize once, as a deliberate user action,
since that is a one-time choice, not a live update.

Joe: "we gotta setup a rule for any widgets going forward... they shouldnt be changing
sizes... thats too obtrusive."

## Sanctioned exceptions

`transform: scale(1.04)` is a drag pick-up affordance, not a content-driven resize, at both of
its sites: `.tile.dragging` in `src/styles/tile.css` (the live strip) and `.wsf-clone` in
`src/styles/settings.css` (the settings-preview drag clone). Both stay. Any new drag pick-up
affordance must be added here too.

**A floating overlay's resize grip** (`#overlay-resize`, `src/styles/base.css`) is a continuous,
deliberate user action, so it is neither a live change nor a one-time config toggle. Chosen by Joe
on 2026-08-07 over fixed scale steps. The invariant the rule actually protects still holds in full:
content never changes an overlay's size, only the user's grip does, and the widget's declared
`overlay`/`flyout` dims are enforced natively as the minimum it can be shrunk to.

## Reserving width for numbers

`min-width: 3ch` alone does NOT hold a number's width. `ch` is the width of the "0" glyph, so it
only guarantees anything once `font-variant-numeric: tabular-nums` forces every digit to that
width. Measured 2026-08-05: even with both, plain `3ch` came out ~0.03px under three real glyphs in
Segoe UI Variable, so a 3-digit value still nudged the box wider. The shipped rule uses
`calc(3ch + 2px)`. Verify a new numeric tile with `getBoundingClientRect()` at min and max values,
never by eye.

## Where sizes live

Flyout dims are declared per widget in the `flyout` field of the `TaskbarWidget` contract,
`src/shared/widget.ts`. Overlay dims live in the sibling `overlay` field; `overlayDims()` resolves
`overlay ?? flyout`, and a widget declaring neither cannot be placed as a floating overlay.
