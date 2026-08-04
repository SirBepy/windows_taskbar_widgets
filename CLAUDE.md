# windows_taskbar_widgets

## Fixed-size rule

Once a widget tile or flyout overlay is mounted, its footprint must not change size: not on
hover, not on content change, not on toggling a config option. A widget whose content length
can vary (a process list, a drive list) must size for its MAX case at declare time, not resize
live. Overflow beyond that max scrolls inside the fixed box (`#flyout`'s `overflow-y: auto`),
it never grows the window.

Joe: "we gotta setup a rule for any widgets going forward... they shouldnt be changing
sizes... thats too obtrusive."

## Sanctioned exception

`.tile.dragging`'s `transform: scale(1.04)` in `src/styles/base.css` is a drag pick-up
affordance, not a content-driven resize. It stays.

## Where sizes live

Flyout dims are declared per widget in the `flyout` field of the `TaskbarWidget` contract,
`src/shared/widget.ts`.
